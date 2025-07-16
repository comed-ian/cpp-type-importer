use binaryninja::binary_view::BinaryViewExt;
use binaryninja::command::{register_command, Command};
use binaryninja::logger::Logger;
use binaryninja::rc::Ref;
use binaryninja::types::{
    BaseStructure, EnumerationBuilder, FunctionParameter, MemberAccess, MemberScope,
    NamedTypeReference, NamedTypeReferenceClass, StructureBuilder, Type,
};
use binaryninja::{architecture::Architecture, binary_view::BinaryView};
use log::{error, info, LevelFilter};
use regex::Regex;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

// TODO
// 1. Multi-level inheritance
// 2. Templated Typedefs
// 3. Conflicting vtable function names (e.g., MyMethod)

/// Maps C++ primitive type names to Binary Ninja types
///
/// # Arguments
/// * `s` - The C++ primitive type name as a string
///
/// # Returns
/// * `Some(Ref<Type>)` - Binary Ninja type reference if the type is primitive
/// * `None` - If the type is not a recognized primitive
fn is_primitive(s: &str) -> Option<Ref<Type>> {
    match s {
        "char" => Some(Type::int(1, true)),
        "unsigned char" => Some(Type::int(1, false)),
        "int8_t" => Some(Type::int(1, true)),
        "uint8_t" => Some(Type::int(1, false)),
        "int16_t" => Some(Type::int(2, true)),
        "uint16_t" => Some(Type::int(3, false)),
        "int32_t" => Some(Type::int(4, true)),
        "uint32_t" => Some(Type::int(4, false)),
        "int" => Some(Type::int(4, true)),
        "unsigned int" => Some(Type::int(4, false)),
        "int64_t" => Some(Type::int(8, true)),
        "uint64_t" => Some(Type::int(8, false)),
        "bool" => Some(Type::bool()),
        "void" => Some(Type::void()),
        _ => None,
    }
}

pub fn get_type_width_by_name(name: &str, bv: &BinaryView) -> Option<u64> {
    let id = bv.type_id_by_name(name)?;
    // Get the width of the base vtable. This works with `NamedTypedReference`
    // because the underlying type is a pointer. BEWARE, this does not work
    // if it was a defined structure.
    let typ = bv.type_by_id(&id)?;
    Some(typ.width())
}

/// Represents a C++ enum with values and size specification
///
/// This structure stores enum name, underlying type size, and value mappings
/// to support Binary Ninja enum type creation.
#[derive(Debug, Clone)]
pub struct Enum {
    /// The name of the enum type
    name: String,
    /// The size of the underlying type in bytes
    size: u8,
    /// List of enum values as (name, numeric_value) pairs
    values: Vec<(String, u64)>,
    /// Namespace path for this enum
    namespace_path: Vec<String>,
}

impl<'a> Enum {
    /// Creates a new enum from its definition
    ///
    /// # Arguments
    /// * `name` - The enum name
    /// * `size` - Size in bytes of the underlying type
    /// * `body` - The enum body containing value definitions
    ///
    /// # Returns
    /// A new `Enum` instance with parsed values
    pub fn new(name: &str, size: u8, body: &str, namespace_path: Vec<String>) -> Self {
        let mut values = Vec::new();
        // Enum values increment from the prior value if not specified,
        // with the default being 0
        let mut current_value = 0u64;

        for line in body.lines() {
            // Each line has the form `VALUE,` or `VALUE=X,`
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let line = line.trim_end_matches(',');

            if let Some(eq_pos) = line.find('=') {
                // Parse explicit assignment like "VALUE=1"
                let name = line[..eq_pos].trim().to_string();
                let value_str = line[eq_pos + 1..].trim();

                // Parse the numeric value
                if let Ok(assigned_value) = value_str.parse::<u64>() {
                    current_value = assigned_value;
                } else {
                    // Default to current_value
                    log::warn!(
                        "Could not parse enum value '{}', using {}",
                        value_str,
                        current_value
                    );
                }

                values.push((name, current_value));
            } else {
                // No explicit assignment, use current_value
                values.push((line.to_string(), current_value));
            }

            current_value += 1;
        }

        Self {
            name: name.to_string(),
            size,
            values,
            namespace_path,
        }
    }

    /// Defines the enum in Binary Ninja's type system
    ///
    /// # Arguments
    /// * `bv` - Binary Ninja binary view reference
    ///
    /// # Returns
    /// `true` if the enum was successfully defined
    /// Gets the full name including namespace prefix
    fn get_full_name(&self) -> String {
        if self.namespace_path.is_empty() {
            self.name.clone()
        } else {
            format!("{}::{}", self.namespace_path.join("::"), self.name)
        }
    }

    pub fn define(&self, bv: &BinaryView) -> bool {
        // Create an enumeration builder
        let mut builder = EnumerationBuilder::new();

        // Add each enum value to the builder with its correct numeric value
        for (name, value) in &self.values {
            builder.insert(name, *value);
        }

        let enumeration = builder.finalize();

        // Create the enum type with the specified width, default to 4 bytes
        let width = std::num::NonZeroUsize::new(self.size as usize)
            .unwrap_or_else(|| std::num::NonZeroUsize::new(4).unwrap());
        let enum_type = Type::enumeration(&enumeration, width, false);

        // Construct full name with namespace prefix
        let full_name = self.get_full_name();
        bv.define_user_type(&full_name, &enum_type);
        true
    }
}

/// Represents a C++ typedef definition.
///
/// This can be either a basic structure or a templated structure. Because
/// Binary Ninja's API does not expose a `Type::typedef`, use the
/// `TypeParser` to parse a literal string.
#[derive(Debug)]
pub struct Typedef {
    name: String,
    typ: String,
    depth: u8,
    namespace_path: Vec<String>,
}

impl<'a> Typedef {
    pub fn new(def: &str, namespace_path: Vec<String>) -> Self {
        let (typ, name, depth) =
            parse_member_definition(def).expect("Could not parse typedef definition");
        Self {
            name,
            typ,
            depth,
            namespace_path,
        }
    }

    /// Gets the full name including namespace prefix
    fn get_full_name(&self) -> String {
        if self.namespace_path.is_empty() {
            self.name.clone()
        } else {
            format!("{}::{}", self.namespace_path.join("::"), self.name)
        }
    }

    pub fn define(&self, bv: &'a BinaryView) -> bool {
        let target_type = if let Some(tt) = is_primitive(&self.typ) {
            tt
        } else {
            Member::define_type(&self.typ, self.depth, bv)
        };
        log::info!("Got typedef target type: {}", target_type);

        // Construct full name with namespace prefix
        let full_name = self.get_full_name();
        bv.define_user_type(&full_name, &target_type);
        true
    }
}

/// Represents a C++ template definition with type parameters
///
/// This structure stores the template name, parameter names, and body content
/// to support template instantiation with concrete types.
#[derive(Debug, Clone)]
pub struct Template {
    /// The name of the template (e.g., "vector" for std::vector)
    name: String,
    /// List of template parameter names (e.g., ["T", "Allocator"])
    typenames: Vec<String>,
    /// The template body containing member definitions
    body: String,
    /// Namespace path for this template
    namespace_path: Vec<String>,
}

impl<'a> Template {
    /// Creates a new template from its definition
    ///
    /// # Arguments
    /// * `def` - The template declaration line, like `template template_name`
    /// or simply `template_name`
    /// * `body` - The template body containing member definitions
    /// * `typenames` - List of template parameter names parsed from the definition line
    ///
    /// # Returns
    /// A new `Template` instance
    pub fn new(def: &str, body: &str, typenames: Vec<String>, namespace_path: Vec<String>) -> Self {
        let name = parse_name(def).expect(&format!("Could not parse name from {def}"));
        Self {
            name,
            typenames,
            body: body.to_string(),
            namespace_path,
        }
    }

    /// Instantiates the template with concrete types in Binary Ninja
    ///
    /// # Arguments
    /// * `typenames` - Concrete type names to substitute for template parameters,
    /// ordered according to the required substitution order in `self.typenames`
    /// * `bv` - Binary Ninja binary view reference
    ///
    /// # Panics
    /// Panics if the number of provided type names doest not match template parameters
    pub fn define<'b>(&self, typenames: Vec<String>, bv: &'a BinaryView) {
        assert_eq!(
            typenames.len(),
            self.typenames.len(),
            "Provided typenames length does not match expected typenames length"
        );
        let mut members = Vec::<Member>::new();
        for member in self.body.lines() {
            if member.trim() == "" {
                continue;
            }
            log::debug!("Defining member: {}", member);
            // Create a new member and provide the typenames to swap in case
            // the given member uses a typename
            members.push(Member::new(
                member,
                bv,
                Some(&self.typenames),
                Some(&typenames),
                &self.namespace_path,
            ));
        }
        // `self.name` is simply the name of the templated structure. Add
        // `<typename1, typename2, ...>` to distinguish this particular
        // instantiation.
        let mut name = self.name.clone();
        name.push('<');
        name.push_str(&typenames.join(", "));
        name.push('>');
        Structure::new_from_members(name, members, 0, self.namespace_path.clone()).define(bv);
    }

    /// Gets the full name including namespace prefix
    fn get_full_name(&self) -> String {
        if self.namespace_path.is_empty() {
            self.name.clone()
        } else {
            format!("{}::{}", self.namespace_path.join("::"), self.name)
        }
    }
}

/// Represents a C++ struct with members and Binary Ninja type information
///
/// This structure stores the struct name, member definitions, and offset information
/// to support Binary Ninja struct type creation.
#[derive(Debug)]
pub struct Structure {
    /// The name of the struct
    name: String,
    /// List of struct members
    members: Vec<Member>,
    /// Base offset for the struct
    offset: u16,
    /// Whether the struct is packed (no padding)
    packed: bool,
    /// Namespace path for this structure
    namespace_path: Vec<String>,
}

impl<'a> Structure {
    /// Creates a new structure from its definition
    ///
    /// # Arguments
    /// * `def` - The struct declaration line, like `struct struct_name`
    /// or simply `struct_name`
    /// * `body` - The struct body containing member definitions
    /// * `bv` - Binary Ninja binary view reference
    ///
    /// # Returns
    /// A new `Structure` instance
    pub fn new<'b>(def: &str, body: &str, bv: &'a BinaryView, namespace_path: Vec<String>) -> Self {
        let name = parse_name(def).expect(&format!("Could not parse definition {def} for name"));

        // Check for packed attribute in the definition
        let packed = def.contains("__attribute__((packed))");

        let mut members = vec![];
        for member in body.lines() {
            // Create a new member with no templated fields
            members.push(Member::new(member, bv, None, None, &namespace_path));
        }

        Self {
            name,
            members,
            offset: 0,
            packed,
            namespace_path,
        }
    }

    /// Creates a new structure from pre-parsed members. Useful for coercing
    /// other types into a `Structure` for easy definition, such as [`Class`]
    /// and [`Template`].
    ///
    /// # Arguments
    /// * `name` - The pre-parsed struct name
    /// * `members` - Pre-parsed member list
    /// * `offset` - Base offset for the struct
    ///
    /// # Returns
    /// A new `Structure` instance
    pub fn new_from_members(
        name: String,
        members: Vec<Member>,
        offset: u16,
        namespace_path: Vec<String>,
    ) -> Self {
        Self {
            name,
            members,
            offset,
            packed: false,
            namespace_path,
        }
    }

    /// Defines the structure in Binary Ninja's type system
    ///
    /// # Arguments
    /// * `bv` - Binary Ninja binary view reference
    ///
    /// # Returns
    /// `true` if the structure was successfully defined
    /// TODO migrate definition
    /// TODO warn and return false anything fails
    pub fn define<'b>(&mut self, bv: &'a BinaryView) -> bool {
        let mut builder = StructureBuilder::new();

        // Set packed flag if the structure is packed
        if self.packed {
            builder.packed(true);
        }

        for m in self.members.iter_mut() {
            m.define(None, &mut builder, bv);
        }
        let s = Type::structure(&builder.finalize());
        let full_name = self.get_full_name();
        log::debug!(
            "Defining {} structure {}",
            if self.packed { "(packed)" } else { "" },
            full_name
        );
        bv.define_user_type(&full_name, &s);
        true
    }

    /// Gets the full name including namespace prefix
    fn get_full_name(&self) -> String {
        if self.namespace_path.is_empty() {
            self.name.clone()
        } else {
            format!("{}::{}", self.namespace_path.join("::"), self.name)
        }
    }
}

/// Represents a C++ class with virtual table, members, and inheritance
///
/// This structure supports C++ class features including virtual methods,
/// member variables, and inheritance from base classes.
#[derive(Debug)]
pub struct Class {
    /// The name of the class
    name: String,
    /// Virtual table methods with optional override information
    vtable_methods: Vec<(Member, Option<String>)>,
    /// Member variables with optional override information
    member_variables: Vec<(Member, Option<String>)>,
    /// List of base class names for inheritance
    base_classes: Vec<String>,
    /// Namespace path for this class
    namespace_path: Vec<String>,
}

impl<'a> Class {
    /// Creates a new class from its definition
    ///
    /// # Arguments
    /// * `def` - The class declaration line with inheritance,
    /// e.g. `class class_name : parent1, parent2`
    /// * `body` - The class body containing method and member definitions
    /// * `bv` - Binary Ninja binary view reference
    ///
    /// # Returns
    /// A new `Class` instance
    pub fn new(def: &str, body: &str, bv: &'a BinaryView, namespace_path: Vec<String>) -> Self {
        // Parse class name and inheritance with regex
        // TODO parse inherited classes differently, inherited classes could be templated
        let class_regex = Regex::new(r"class\s+(\w+)(?:\s*:\s*(.+))?").unwrap();
        let (name, base_classes) = if let Some(captures) = class_regex.captures(def) {
            let class_name = captures.get(1).unwrap().as_str().to_string();
            let base_classes = if let Some(inheritance) = captures.get(2) {
                // Parse inherited classes (split by comma or space)
                inheritance
                    .as_str()
                    .split_whitespace()
                    .filter(|s| {
                        !s.is_empty() && *s != "public" && *s != "private" && *s != "protected"
                    })
                    .map(|s| s.trim_end_matches(',').to_string())
                    .collect()
            } else {
                Vec::new()
            };
            (class_name, base_classes)
        } else {
            // Fallback to existing `parse_name` logic
            let name = parse_name(def).expect(&format!("Could not parse class name from {def}"));
            (name, Vec::new())
        };

        // Forward-declare the class as a structure so it can be referenced in constructor signatures
        let forward_decl = Type::structure(&StructureBuilder::new().finalize());
        let full_name = if namespace_path.is_empty() {
            name.clone()
        } else {
            format!("{}::{}", namespace_path.join("::"), name)
        };
        bv.define_user_type(&full_name, &forward_decl);

        let mut vtable_methods = Vec::new();
        let mut member_variables = Vec::new();
        let mut in_vtable = true;

        // Body contains a list of vtable methods (including any overrides) followed by a single
        // line `// ; end vtable` denoting the end of the vtable and start of members (including
        // overrides).
        for line in body.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            // Check for end of vtable marker
            if line.contains("// ; end vtable") {
                in_vtable = false;
                continue;
            }

            if in_vtable {
                // Parse vtable method
                if let Some((method, override_info)) = Self::parse_vtable_method(line, bv, &name) {
                    vtable_methods.push((method, override_info));
                }
            } else {
                // Parse member variable
                // TODO include offset information in this
                let override_regex = Regex::new(r"//\s*;\s*override\s+(.+?);").unwrap();
                let override_info = if let Some(captures) = override_regex.captures(line) {
                    Some(captures.get(1).unwrap().as_str().to_string())
                } else {
                    None
                };

                // Remove override comment from the line
                let clean_line = override_regex.replace(line, "").trim().to_string();
                member_variables.push((
                    Member::new(&clean_line, bv, None, None, &namespace_path),
                    override_info,
                ));
            }
        }

        Self {
            name,
            vtable_methods,
            member_variables,
            base_classes,
            namespace_path,
        }
    }

    /// Parses a virtual table method definition with offset and override information
    ///
    /// # Arguments
    /// * `line` - The method definition line
    /// * `bv` - Binary Ninja binary view reference
    /// * `class_name` - The class name for this pointer injection
    ///
    /// # Returns
    /// `Some((Member, Option<String>))` if parsing succeeds, `None` otherwise
    fn parse_vtable_method(
        line: &str,
        bv: &'a BinaryView,
        class_name: &str,
    ) -> Option<(Member, Option<String>)> {
        // Parse offset comment (`// ; offset=XX`) for `*this` with regex
        let offset_regex = Regex::new(r"//\s*;\s*offset=(-?\d+)").unwrap();
        let this_offset = if let Some(captures) = offset_regex.captures(line) {
            let offset_str = captures.get(1).unwrap().as_str();
            if offset_str == "-1" {
                None // Static method
            } else {
                offset_str.parse::<usize>().ok()
            }
        } else {
            Some(0) // Default: `*this` is at position 0
        };

        // Parse override comment (`// ; override ret function_name(args, ...);`) with regex
        let override_regex = Regex::new(r"//\s*;\s*override\s+(.+?);").unwrap();
        let override_info = if let Some(captures) = override_regex.captures(line) {
            Some(captures.get(1).unwrap().as_str().to_string())
        } else {
            None
        };

        // Remove both offset and override comments from the line
        let clean_line = offset_regex.replace(line, "");
        let clean_line = override_regex.replace(&clean_line, "").trim().to_string();

        // Parse method signature and inject this pointer
        if let Some(member) = Self::parse_method_signature(&clean_line, bv, this_offset, class_name)
        {
            Some((member, override_info))
        } else {
            None
        }
    }

    /// Parses a method signature (stripped of any comments)
    /// and injects `*this` at specified offset
    ///
    /// # Arguments
    /// * `line` - The method signature line
    /// * `bv` - Binary Ninja binary view reference
    /// * `this_offset` - Optional offset for `*this` injection
    /// * `class_name` - The class name for `*this` type
    ///
    /// # Returns
    /// `Some(Member)` if parsing succeeds, `None` otherwise
    fn parse_method_signature(
        line: &str,
        bv: &'a BinaryView,
        this_offset: Option<usize>,
        class_name: &str,
    ) -> Option<Member> {
        let line = line.trim();

        // Create this pointer type string for injection
        let this_param = format!("{class_name}* this");

        // Check for destructor: ~ClassName()
        if line.starts_with('~') && line.contains(&format!("~{class_name}")) {
            // TODO assumption that there are no more args to a destructor
            let method_signature = if this_offset.is_some() {
                format!("void (*~{class_name})({this_param});")
            } else {
                format!("void (*~{class_name})();")
            };
            return Member::new(&method_signature, bv, None, None, &Vec::new()).into();
        }

        // Check for constructor: ClassName(...)
        if let Some(paren_pos) = line.find('(') {
            let potential_constructor = line[..paren_pos].trim();
            if potential_constructor == class_name {
                let params_str = &line[paren_pos + 1..line.rfind(')').unwrap_or(line.len())];

                // Parse all parameters first
                let mut all_params = if params_str.is_empty() {
                    Vec::new()
                } else if let Some(params) = parse_template_instantiation(params_str) {
                    params
                } else {
                    // Fallback to simple splitting if parsing fails
                    params_str
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .collect()
                };

                // Insert this pointer at the specified offset
                if let Some(offset) = this_offset {
                    let insert_pos = if offset <= all_params.len() {
                        offset
                    } else {
                        all_params.len()
                    };
                    all_params.insert(insert_pos, this_param);
                }

                let method_signature = if all_params.is_empty() {
                    format!("void (*{class_name})();")
                } else {
                    format!("void (*{class_name})({});", all_params.join(", "))
                };

                return Member::new(&method_signature, bv, None, None, &Vec::new()).into();
            }
        }

        // For regular methods, inject `*this` pointer at the specified offset
        if let Some(paren_pos) = line.find('(') {
            let before_paren = &line[..paren_pos];
            let params_str = &line[paren_pos + 1..line.rfind(')').unwrap_or(line.len())];

            // Parse all parameters first
            let mut all_params = if params_str.is_empty() {
                Vec::new()
            } else if let Some(params) = parse_template_instantiation(params_str) {
                params
            } else {
                // Fallback to simple splitting if parsing fails
                params_str
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect()
            };

            // Insert this pointer at the specified offset
            if let Some(offset) = this_offset {
                let insert_pos = if offset <= all_params.len() {
                    offset
                } else {
                    all_params.len()
                };
                all_params.insert(insert_pos, this_param);
            }

            // Extract method name from before_paren
            // TODO return type could contain a templated type with whitespace
            let parts: Vec<&str> = before_paren.trim().split_whitespace().collect();
            let (return_type, method_name) = if parts.len() >= 2 {
                (parts[..parts.len() - 1].join(" "), parts[parts.len() - 1])
            } else {
                ("void".to_string(), parts[0])
            };

            let method_signature = if all_params.is_empty() {
                format!("{return_type} (*{method_name})();")
            } else {
                format!("{return_type} (*{method_name})({});", all_params.join(", "))
            };

            Member::new(&method_signature, bv, None, None, &Vec::new()).into()
        } else {
            None
        }
    }

    /// Processes virtual table methods for a specific base class
    ///
    /// # Arguments
    /// * `vtable_methods` - List of virtual table methods
    /// * `vtable_builder` - Structure builder for the virtual table
    /// * `base_class` - The base class name
    /// * `process_non_overriding` - Whether to process non-overriding methods
    /// * `bv` - Binary Ninja binary view reference
    fn process_vtable_methods_for_base(
        vtable_methods: &[(Member, Option<String>)],
        vtable_builder: &mut StructureBuilder,
        base_class: &str,
        process_non_overriding: bool,
        bv: &'a BinaryView,
    ) {
        for (method, override_info) in vtable_methods {
            // TODO move this to Member::define
            if let Member::Function { .. } = method {
                if let Some(override_str) = override_info {
                    // Check if this override targets the current base class
                    log::info!("Handling override: {}", override_str);
                    // TODO what about multiple levels of inheritance?
                    if Self::is_override_for_base_class(override_str, base_class) {
                        // Parse the override information to find the target method
                        if let Some(offset) =
                            Self::parse_override_offset(override_str, base_class, bv)
                        {
                            // Override at specific offset
                            method.define(Some(offset), vtable_builder, bv);
                        }
                    }
                } else if process_non_overriding {
                    // No override, add to vtable only if processing non-overriding methods
                    method.define(None, vtable_builder, bv);
                }
            }
        }
    }

    /// Checks if an override string targets a specific base class
    ///
    /// # Arguments
    /// * `override_str` - The override specification string, like
    /// `void (* base_vtable::fn)(struct base* this);`
    /// * `base_class` - The base class name to check
    ///
    /// # Returns
    /// `true` if the override targets the base class
    /// TODO what about multiple levels of inheritance
    fn is_override_for_base_class(override_str: &str, base_class: &str) -> bool {
        // Check if the override string contains the base class vtable name
        let base_vtable_name = format!("{}_vtable", base_class);
        override_str.contains(&base_vtable_name)
    }

    /// Parses the offset for a method override in a base class virtual table
    ///
    /// # Arguments
    /// * `override_str` - The override specification string
    /// * `base_class` - The base class name
    /// * `bv` - Binary Ninja binary view reference
    ///
    /// # Returns
    /// `Some(u64)` with the offset if found, `None` otherwise
    fn parse_override_offset(
        override_str: &str,
        base_class: &str,
        bv: &'a BinaryView,
    ) -> Option<u64> {
        // Parse override string like `void (* base_vtable::fn)(struct base* this);`
        // Look for the base class vtable and method name
        let base_vtable_name = format!("{}_vtable", base_class);

        // Find the vtable type and look for the method
        if let Some(base_vtable_type_id) = bv.type_id_by_name(&base_vtable_name) {
            if let Some(base_vtable_type) = bv.type_by_id(&base_vtable_type_id) {
                // Get the structure members to find the method offset
                if let Some(structure) = base_vtable_type.get_structure() {
                    // Parse the method name from the override string. `cleaned_override`
                    // should look like `return_type (* fn)(args, ...)`
                    if let Some(cleaned_override) =
                        Self::extract_method_name_from_override(override_str)
                    {
                        // Find the method in the base vtable structure by reconstructing the signature
                        for (i, member) in structure.members().iter().enumerate() {
                            // Get member type contents, which does not include the function name,
                            // just the return value, `(*)`, and arguments
                            let mut contents = member.ty.contents.to_string();

                            // Find `(*)` and insert the member name after the `*`
                            if let Some(star_pos) = contents.find("(*)") {
                                contents.insert_str(star_pos + 2, &format!(" {}", member.name));
                            }

                            if cleaned_override == contents {
                                log::info!(
                                    "Found override for '{}' in {}",
                                    cleaned_override,
                                    base_class
                                );
                                // Return the offset in bytes (assuming pointer size)
                                let pointer_size = bv
                                    .default_arch()
                                    .expect("Could not find default arch")
                                    .address_size();
                                return Some((i as u64) * (pointer_size as u64));
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Extracts the method name from an override specification
    ///
    /// # Arguments
    /// * `override_str` - The override specification string, like
    /// `void (* base_vtable::fn)(struct base* this)`
    ///
    /// # Returns
    /// `Some(String)` with the cleaned method name (e.g., `void (* fn)(struct base* this)`),
    /// `None` if parsing fails
    /// TODO what about mutli-level vtables
    fn extract_method_name_from_override(override_str: &str) -> Option<String> {
        // Parse override string like `void (* base_vtable::fn)(struct base* this);`
        // Remove the inherited `base_vtable::` prefix while keeping the rest
        let regex = Regex::new(r"\w+_vtable::").unwrap();
        let cleaned = regex.replace_all(override_str, "");
        Some(cleaned.to_string())
    }

    /// Extracts the member name from an override specification for a base class
    ///
    /// # Arguments
    /// * `override_str` - The override specification string, like
    /// `bool base_class::val1;`
    /// * `base_class` - The base class name
    ///
    /// # Returns
    /// `Some(String)` with the cleaned member name (e.g., `type_name member_name`)
    fn extract_member_from_override(override_str: &str, base_class: &str) -> Option<String> {
        // For member overrides, remove the `base_class::` prefix from anywhere in the string
        let prefix = format!("{}::", base_class);
        let cleaned = override_str.replace(&prefix, "");
        Some(cleaned)
    }

    /// Parses the offset for a member override in base classes
    ///
    /// # Arguments
    /// * `override_str` - The override specification string
    /// * `base_classes` - List of base class names
    /// * `bv` - Binary Ninja binary view reference
    ///
    /// # Returns
    /// `Some((String, u64))` with base class name and offset if found
    fn parse_member_override_offset(
        override_str: &str,
        base_classes: &[String],
        bv: &'a BinaryView,
    ) -> Option<(String, u64)> {
        // Try to find the member in each base class
        for base_class in base_classes {
            if let Some(cleaned_override) =
                Self::extract_member_from_override(override_str, base_class)
            {
                // `cleaned_override` should be `type_name member_name`
                if let Some(base_type_id) = bv.type_id_by_name(base_class) {
                    if let Some(base_type) = bv.type_by_id(&base_type_id) {
                        if let Some(structure) = base_type.get_structure() {
                            // Find the member in the base class structure
                            for member in structure.members() {
                                // Get type contents
                                let mut member_type_contents = member.ty.contents.to_string();
                                // Check to see if this is a function member
                                let check_against =
                                    if let Some(star_pos) = member_type_contents.find("(*)") {
                                        // `member_type_contents` contains the return value, `(*)`,
                                        // and the arguments. Does not contain the function name, so
                                        // insert it
                                        member_type_contents
                                            .insert_str(star_pos + 2, &format!(" {}", member.name));
                                        member_type_contents
                                    } else {
                                        // Basic member type
                                        format!("{member_type_contents} {}", member.name)
                                    };

                                if cleaned_override == check_against {
                                    log::info!(
                                        "Found member override '{}' in base class '{}'",
                                        cleaned_override,
                                        base_class
                                    );
                                    return Some((base_class.clone(), member.offset));
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Defines the class and its virtual tables in Binary Ninja's type system
    ///
    /// # Arguments
    /// * `bv` - Binary Ninja binary view reference
    ///
    /// # Returns
    /// `true` if the class was successfully defined
    pub fn define(&self, bv: &'a BinaryView) -> bool {
        // Create separate vtables for overriding each base class's
        let mut vtable_names = Vec::new();

        if self.base_classes.is_empty() {
            // No inheritance - create regular vtable
            let vtable_name = format!("{}_vtable", self.name);
            let mut vtable_builder = StructureBuilder::new();

            for (method, _) in &self.vtable_methods {
                if let Member::Function { .. } = method {
                    method.define(None, &mut vtable_builder, bv);
                }
            }

            let vtable_structure = Type::structure(&vtable_builder.finalize());
            bv.define_user_type(&vtable_name, &vtable_structure);
            vtable_names.push(vtable_name);
        } else {
            // Has inheritance - create vtables for each base class
            for (i, base_class) in self.base_classes.iter().enumerate() {
                let vtable_name = format!("{}_vtable_{}", self.name, base_class);
                let base_vtable_name = format!("{}_vtable", base_class);

                let mut vtable_builder = StructureBuilder::new();

                // Add base class vtable as base structure and set proper width
                let mut base_vtable_width = 0u64;
                if let Some(base_vtable_type_id) = bv.type_id_by_name(&base_vtable_name) {
                    let base_vtable_ref = NamedTypeReference::new_with_id(
                        NamedTypeReferenceClass::StructNamedTypeClass,
                        &base_vtable_type_id,
                        &base_vtable_name,
                    );

                    // Get the width of the base vtable.
                    if let Some(width) = get_type_width_by_name(&base_vtable_name, bv) {
                        base_vtable_width = width;
                    }

                    // Set the base structure in the new vtable
                    let base_struct = BaseStructure::new(base_vtable_ref, 0, base_vtable_width);
                    vtable_builder.base_structures(&[base_struct]);

                    // Set the vtable builder width to account for the base vtable
                    vtable_builder.width(base_vtable_width);
                }

                // Process vtable methods for this base class. Handles overrides and adding
                // the class-specific methods to the vtable if this is the first base class
                Self::process_vtable_methods_for_base(
                    &self.vtable_methods,
                    &mut vtable_builder,
                    base_class,
                    i == 0, // Only process methods for the first base class
                    bv,
                );

                // Define the inherited vtable structure
                let vtable_structure = Type::structure(&vtable_builder.finalize());
                bv.define_user_type(&vtable_name, &vtable_structure);
                vtable_names.push(vtable_name);
            }
        }

        // Now create the main class structure
        let mut class_builder = StructureBuilder::new();

        // Step 1: Add base class members using base_structures with proper offset and width
        let mut base_structures = Vec::new();
        let mut cumulative_width = 0u64;

        for base_class in &self.base_classes {
            if let Some(base_type_id) = bv.type_id_by_name(base_class) {
                let base_ref = NamedTypeReference::new_with_id(
                    NamedTypeReferenceClass::StructNamedTypeClass,
                    &base_type_id,
                    base_class,
                );

                // Get the width of this base class
                let base_width = get_type_width_by_name(&base_class, bv).unwrap_or(0);

                let base_struct = BaseStructure::new(base_ref, cumulative_width, base_width);
                base_structures.push(base_struct);

                // Update cumulative width for next base class
                cumulative_width += base_width;
            }
        }

        if !base_structures.is_empty() {
            class_builder.base_structures(&base_structures);
            // Set the class builder width to the cumulative width of all base structures
            class_builder.width(cumulative_width);
        }

        // Step 2: Override base class vtables
        if !self.base_classes.is_empty() {
            let mut current_offset = 0;

            for (i, base_class) in self.base_classes.iter().enumerate() {
                if let Some(vtable_name) = vtable_names.get(i) {
                    // Create vtable pointer
                    let vtable_ptr = Type::pointer(
                        &bv.default_arch().expect("Could not find default arch"),
                        &Type::named_type(&NamedTypeReference::new(
                            NamedTypeReferenceClass::StructNamedTypeClass,
                            vtable_name,
                        )),
                    );

                    // Insert vtable at the start of this base class (current_offset)
                    let vtable_member_name = format!("vtable_{}", base_class);
                    class_builder.insert(
                        vtable_ptr.as_ref(),
                        &vtable_member_name,
                        current_offset,
                        true, // overwrite existing
                        MemberAccess::PublicAccess,
                        MemberScope::NoScope,
                    );

                    // Update offset for next base class using the actual size/width
                    if let Some(width) = get_type_width_by_name(&base_class, bv) {
                        current_offset += width;
                    }
                }
            }
        } else if !self.vtable_methods.is_empty() {
            // Regular class with vtable - insert at offset 0
            if let Some(vtable_name) = vtable_names.get(0) {
                let vtable_ptr = Type::pointer(
                    &bv.default_arch().expect("Could not find default arch"),
                    &Type::named_type(&NamedTypeReference::new(
                        NamedTypeReferenceClass::StructNamedTypeClass,
                        vtable_name,
                    )),
                );
                class_builder.insert(
                    vtable_ptr.as_ref(),
                    "vtable",
                    0,
                    true, // overwrite existing
                    MemberAccess::PublicAccess,
                    MemberScope::NoScope,
                );
            }
        }

        // Step 3: Handle member variable overrides and add member variables specific to this class
        for (member, override_info) in &self.member_variables {
            if let Some(override_str) = override_info {
                // This member overrides a base class member
                if let Some((base_class, base_offset)) =
                    Self::parse_member_override_offset(override_str, &self.base_classes, bv)
                {
                    // Calculate the actual offset by adding the base class offset
                    let mut actual_offset = base_offset;
                    for base in self.base_classes.iter() {
                        if base == &base_class {
                            break;
                        }
                        // Add the size of previous base classes
                        if let Some(width) = get_type_width_by_name(&base, bv) {
                            actual_offset += width;
                        }
                    }

                    // Insert the overriding member at the calculated offset
                    member.define(Some(actual_offset), &mut class_builder, bv);
                    continue;
                }
            }

            // No override, append normally
            member.define(None, &mut class_builder, bv);
        }

        // Define the class structure
        let class_structure = Type::structure(&class_builder.finalize());
        let full_name = self.get_full_name();
        bv.define_user_type(&full_name, &class_structure);

        true
    }

    /// Gets the full name including namespace prefix
    fn get_full_name(&self) -> String {
        if self.namespace_path.is_empty() {
            self.name.clone()
        } else {
            format!("{}::{}", self.namespace_path.join("::"), self.name)
        }
    }
}

/// Represents different types of C++ class/struct members
///
/// This enum covers basic data members, function pointers, and template members
/// with their respective type information and comments.
#[derive(Debug)]
pub enum Member {
    /// A basic data member with type and name
    Basic {
        /// The member name
        name: String,
        /// Binary Ninja type reference for the member
        typ: Ref<Type>,
        /// Associated comments for the member
        comments: Vec<String>,
    },
    /// A function pointer member with return type and arguments
    Function {
        /// The function name
        name: String,
        /// Binary Ninja type reference for the return type
        ret: Ref<Type>,
        /// List of function arguments as (name, type) pairs
        args: Vec<(String, Ref<Type>)>,
    },
}

impl PartialEq for Member {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Member::Basic {
                    name: name1,
                    typ: typ1,
                    ..
                },
                Member::Basic {
                    name: name2,
                    typ: typ2,
                    ..
                },
            ) => name1 == name2 && typ1.to_string() == typ2.to_string(),
            (
                Member::Function {
                    name: name1,
                    ret: ret1,
                    args: args1,
                    ..
                },
                Member::Function {
                    name: name2,
                    ret: ret2,
                    args: args2,
                    ..
                },
            ) => {
                name1 == name2
                    && ret1.to_string() == ret2.to_string()
                    && args1.len() == args2.len()
                    && args1.iter().zip(args2.iter()).all(
                        |((arg_name1, type1), (arg_name2, type2))| {
                            arg_name1 == arg_name2 && type1.to_string() == type2.to_string()
                        },
                    )
            }
            _ => false, // Different variants are not equal
        }
    }
}

impl Eq for Member {}

impl<'a> Member {
    /// Defines a type with specified pointer depth in Binary Ninja
    ///
    /// # Arguments
    /// * `t` - The type name
    /// * `depth` - Number of pointer indirections
    /// * `bv` - Binary Ninja binary view reference
    ///
    /// # Returns
    /// Binary Ninja type reference for the defined type
    fn define_type(t: &str, depth: u8, bv: &BinaryView) -> Ref<Type> {
        Self::define_type_with_namespace(t, depth, bv, &Vec::new())
    }

    fn define_type_with_namespace(
        t: &str,
        depth: u8,
        bv: &BinaryView,
        current_namespace: &Vec<String>,
    ) -> Ref<Type> {
        log::debug!("Defining type: {}", t);
        let mut typ = if let Some(tt) = is_primitive(t) {
            tt
        } else {
            // Try to resolve the type with namespace resolution
            let resolved_type = Self::resolve_type_name(t, bv, current_namespace);
            if let Some(type_id) = bv.type_id_by_name(&resolved_type) {
                let named_ref = NamedTypeReference::new_with_id(
                    NamedTypeReferenceClass::StructNamedTypeClass,
                    &type_id,
                    &resolved_type,
                );
                // Hack: `Type::named_type(&named_ref)` always returns
                // a reference with width 0, making any member structs
                // (that are not pointers to structs) appear with 0 size.
                // Workaround using `Type::named_type_from_type` after
                // fetching the type with `type_by_ref` using the reference
                // above
                Type::named_type_from_type(
                    named_ref.name(),
                    bv.type_by_ref(&named_ref)
                        .expect("Could not find type by ref")
                        .as_ref(),
                )
            } else {
                // TODO consider returning option and warn
                panic!("Could not find type: {}", resolved_type);
            }
        };
        for _ in 0..depth {
            typ = Type::pointer(
                &bv.default_arch().expect("Could not find core arch"),
                typ.as_ref(),
            );
        }
        typ
    }

    /// Resolves a type name with namespace context
    fn resolve_type_name(
        type_name: &str,
        bv: &BinaryView,
        current_namespace: &Vec<String>,
    ) -> String {
        // If the type name already contains :: it's fully qualified
        if type_name.contains("::") {
            return type_name.to_string();
        }

        // Try to find the type in the current namespace hierarchy
        // Start from the most specific namespace and work outward
        for i in (0..=current_namespace.len()).rev() {
            let namespace_path = &current_namespace[0..i];
            let candidate_name = if namespace_path.is_empty() {
                type_name.to_string()
            } else {
                format!("{}::{}", namespace_path.join("::"), type_name)
            };

            if bv.type_id_by_name(&candidate_name).is_some() {
                return candidate_name;
            }
        }

        // If not found in any namespace, return the original name
        type_name.to_string()
    }

    /// Creates a new member from its definition string
    ///
    /// # Arguments
    /// * `def` - The member definition string, e.g., `type_name** type`
    /// * `bv` - Binary Ninja binary view reference
    /// * `template_members` - Optional template parameter names
    /// * `template_defs` - Optional template parameter definitions
    /// * `current_namespace` - Current namespace context for type resolution
    ///
    /// # Returns
    /// A new `Member` instance
    fn new(
        def: &str,
        bv: &BinaryView,
        template_members: Option<&Vec<String>>,
        template_defs: Option<&Vec<String>>,
        current_namespace: &Vec<String>,
    ) -> Self {
        let (typ, name, depth) =
            parse_member_definition(def).expect("Could not parse member definition");
        log::info!(
            "Got member definition type={}, name={}, depth={}",
            typ,
            name,
            depth
        );
        if let Some(_) = is_primitive(&typ) {
            let typ = Self::define_type(&typ, depth, bv);
            return Member::Basic {
                name,
                typ,
                comments: vec![],
            };
        } else {
            let mut def = def.to_string();
            // Try and replace templated member types, if they exist
            if let Some(t_members) = template_members {
                let t_defs = template_defs.unwrap();
                let mut tokens = parse_template_member_definition(&def);

                // Replace template parameters with their concrete types
                for token in &mut tokens {
                    if let Some(index) = t_members.iter().position(|t| t == token) {
                        *token = t_defs[index].clone();
                    }
                }

                // Reconstruct the type string with substituted parameters
                def = tokens.join("");
                log::info!("Instantiated templated type: {}", def);
            }
            // Try to match function definition: `return_type (*name)(args)`
            let func_regex = Regex::new(r"(.*) \(\*(.*)\)\((.*)\)").unwrap();
            if let Some(captures) = func_regex.captures(&def) {
                let return_type = captures.get(1).unwrap().as_str().trim();
                let name = captures.get(2).unwrap().as_str().trim();
                let args = captures.get(3).unwrap().as_str().trim();

                log::info!(
                    "Got function definition: return_type={}, name={}, args={:?}",
                    return_type,
                    name,
                    args
                );
                let (return_type, _, depth) = parse_member_definition(return_type)
                    .expect("Could not parse function member return type");
                let args = parse_template_instantiation(args)
                    .expect("Could not parse function member args");
                let mut defined_args = vec![];
                for a in args {
                    let (typ, name, depth) = parse_member_definition(&a)
                        .expect("Could not parse argument to function definition");
                    defined_args.push((
                        name,
                        Self::define_type_with_namespace(&typ, depth, bv, current_namespace),
                    ));
                }

                return Member::Function {
                    name: name.to_string(),
                    ret: Self::define_type_with_namespace(
                        &return_type,
                        depth,
                        bv,
                        current_namespace,
                    ),
                    args: defined_args,
                };
            } else {
                // Not a function definition
                let (typ, name, depth) =
                    parse_member_definition(&def).expect(&format!("Could not parse {def}"));
                let typ = Self::define_type_with_namespace(&typ, depth, bv, current_namespace);
                return Member::Basic {
                    name,
                    typ,
                    comments: vec![],
                };
            }
        }
    }
    pub fn define<'b>(
        &self,
        offset: Option<u64>,
        builder: &mut StructureBuilder,
        bv: &'a BinaryView,
    ) {
        match self {
            Member::Basic {
                name,
                typ,
                comments: _,
            } => {
                // Simply append basic members
                log::debug!("Adding member: {}", name);
                if let Some(o) = offset {
                    builder.insert(
                        typ.as_ref(),
                        &name.clone(),
                        o,
                        true, // overwrite existing
                        MemberAccess::PublicAccess,
                        MemberScope::NoScope,
                    );
                } else {
                    builder.append(
                        typ.as_ref(),
                        &name.clone(),
                        MemberAccess::PublicAccess,
                        MemberScope::NoScope,
                    );
                }
            }
            Member::Function { name, ret, args } => {
                // Create a function type and pointer to that function
                log::debug!("Adding function: {}", name);
                let mut v = vec![];
                for (arg_name, arg_type) in args {
                    v.push(FunctionParameter::new(
                        arg_type.clone(),
                        arg_name.clone(),
                        None,
                    ));
                }
                // Create function with return value, arguments, and `false`
                // indicating no variable arguments
                // TODO include variable arguments
                let func = Type::function(ret.as_ref(), v, false);
                let func = Type::pointer(
                    &bv.default_arch().expect("Could not find default arch"),
                    func.as_ref(),
                );
                if let Some(o) = offset {
                    builder.insert(
                        func.as_ref(),
                        &name.clone(),
                        o,
                        true, // overwrite existing
                        MemberAccess::PublicAccess,
                        MemberScope::NoScope,
                    );
                } else {
                    builder.append(
                        func.as_ref(),
                        &name.clone(),
                        MemberAccess::PublicAccess,
                        MemberScope::NoScope,
                    );
                }
            }
        }
    }
}

/// Parses a template member definition into individual tokens
///
/// # Arguments
/// * `s` - The template member definition string, like
/// `struct_name<type1, type2>** member_name`
///
/// # Returns
/// Vector of tokens from the definition
fn parse_template_member_definition(s: &str) -> Vec<String> {
    let mut curr = String::new();
    let mut typenames = Vec::<String>::new();
    for c in s.chars() {
        match c {
            '<' | '>' | '*' | ',' | ' ' | '(' | ')' => {
                if !curr.is_empty() {
                    typenames.push(curr);
                    curr = String::new();
                }
                typenames.push(c.to_string());
            }
            _ => curr.push(c),
        }
    }

    // get last item if not empty
    if !curr.is_empty() {
        typenames.push(curr);
    }

    typenames
}

/// Parses template instantiation like `temp<type1, type2<type3, type4>>`
///
/// The input should be the text within the opening `<` and closing `>`.
/// Handles nested template arguments with proper bracket matching. Used for
/// finding the types to substitute into generic typenames when instantiating
/// a templated structure or class.
///
/// # Arguments
/// * `s` - The template instantiation string (contents between angle brackets)
///
/// # Returns
/// `Some(Vec<String>)` with parsed type arguments, `None` if parsing fails
fn parse_template_instantiation(s: &str) -> Option<Vec<String>> {
    let mut curr = String::new();
    let mut typenames = Vec::<String>::new();
    let mut stack = vec![];
    for c in s.chars() {
        match c {
            '<' => {
                stack.push('<');
            }
            '>' => {
                stack.pop();
                if stack.is_empty() {
                    curr.push(c);
                    typenames.push(curr);
                    curr = String::new();
                    continue;
                }
            }
            ',' => {
                if curr.is_empty() {
                    continue;
                }
                if stack.is_empty() {
                    typenames.push(curr);
                    curr = String::new();
                    continue;
                }
            }
            ' ' => {
                if curr.is_empty() {
                    continue;
                }
            }
            _ => (),
        }
        curr.push(c);
    }

    assert!(
        stack.is_empty(),
        "Could not find balanced < and > in template instantiation"
    );

    // get last item if not empty
    let curr = curr.trim().to_string();
    if !curr.is_empty() {
        typenames.push(curr);
    }

    Some(typenames)
}

/// Parses template parameter names from template definition
///
/// Extracts template parameter names from the contents between angle brackets.
/// Filters out keywords like 'typename' and 'class'. Used to find the generic
/// typenames when parsing the definition of a template struct or class.
///
/// # Arguments
/// * `s` - The template definition string (contents between angle brackets), like
/// `typename1, typename2`.
///
/// # Returns
/// `Some(Vec<String>)` with template parameter names, `None` if parsing fails
fn parse_template_definition(s: &str) -> Option<Vec<String>> {
    let mut curr = String::new();
    let mut typenames = Vec::<String>::new();
    for c in s.chars() {
        match c {
            ' ' | ',' => {
                if curr != "typename".to_string() && curr != "class".to_string() && !curr.is_empty()
                {
                    typenames.push(curr);
                }
                curr = String::new();
            }
            _ => curr.push(c),
        }
    }

    // get last item if not empty
    let curr = curr.trim().to_string();
    if !curr.is_empty() {
        typenames.push(curr);
    }

    Some(typenames)
}

/// Parses the name from a class, struct, template, or enum definition
///
/// Extracts the type name from definitions like "struct MyStruct" or "class MyClass"
/// by stripping the preceding type identifier and any trailing attributes.
///
/// # Arguments
/// * `def` - The definition string, e.g., `struct MyStruct` or `struct __attribute__((packed)) MyStruct`
///
/// # Returns
/// `Some(String)` with the parsed name, `None` if parsing fails
fn parse_name(def: &str) -> Option<String> {
    let mut s = def.trim();

    // Strip type keywords
    if s.starts_with("struct ") {
        s = s.strip_prefix("struct ")?;
    } else if s.starts_with("class ") {
        s = s.strip_prefix("class ")?;
    } else if s.starts_with("enum ") {
        s = s.strip_prefix("enum ")?;
    } else if s.starts_with("template ") {
        s.strip_prefix("template ")?;
    }

    // Handle attributes that come after the type keyword but before the name
    // e.g., "struct __attribute__((packed)) MyStruct"
    if s.starts_with("__attribute__") {
        // Find the end of the attribute and skip it
        if let Some(end) = s.find("))") {
            s = s[end + 2..].trim();
        }
    }

    Some(s.to_string())
}

/// Determines the pointer depth and extracts the suffix from a type definition
///
/// # Arguments
/// * `def` - The type definition string
///
/// # Returns
/// A tuple of (pointer_depth, remaining_suffix)
fn get_pointer_depth(def: &str) -> (u8, String) {
    let mut depth = 0u8;
    let mut suffix = String::new();
    for (i, c) in def.chars().rev().enumerate() {
        match c {
            '*' => depth += 1,
            ' ' => continue,
            _ => {
                suffix = def[def.len() - i..].to_string();
                break;
            }
        }
    }
    (depth, suffix)
}

/// Parses a member definition into type, name, and pointer depth
///
/// # Arguments
/// * `def` - The member definition string
///
/// # Returns
/// `Some((type, name, pointer_depth))` if parsing succeeds, `None` otherwise
fn parse_member_definition(def: &str) -> Option<(String, String, u8)> {
    let mut def = def.trim();
    if let Some(trimmed) = def.strip_suffix(";") {
        def = trimmed;
    }
    let mut def = def.trim();
    let name = parse_member_name(def).unwrap_or("".to_string());
    def = def.strip_suffix(&name).unwrap();
    let (depth, suffix) = get_pointer_depth(def);
    if !suffix.is_empty() {
        def = def.strip_suffix(&suffix).unwrap();
    }

    Some((def.to_string(), name, depth))
}

/// Parses the member name from a member definition
///
/// Extracts the member name from definitions like `struct B* C`.
/// Assumes delineating characters between name and type are `*` and ` `.
///
/// # Arguments
/// * `s` - The member definition string
///
/// # Returns
/// `Some(String)` with the member name, `None` if parsing fails
fn parse_member_name(s: &str) -> Option<String> {
    for (i, c) in s.chars().rev().enumerate() {
        match c {
            '*' | ' ' => {
                return Some(s[s.len() - i..].to_string());
            }
            _ => continue,
        }
    }
    None
}

/// Finds the next significant token in a C++ definition string
///
/// # Arguments
/// * `s` - The string to search
///
/// # Returns
/// `Some((index, character, prefix))` if a token is found, `None` otherwise
fn find_next_token(s: &str) -> Option<(usize, char, &str)> {
    for (i, c) in s.char_indices() {
        if c == '{' || c == ';' || c == '<' || c == '"' || c == '}' {
            return Some((i, c, &s[..i]));
        }
    }
    None
}

/// Finds the closing token that matches the given opening token
///
/// # Arguments
/// * `s` - The string to search
/// * `token` - The opening token character
///
/// # Returns
/// `Some((index, closing_char, prefix))` if a closing token is found, `None` otherwise
fn find_closing_token(s: &str, token: char) -> Option<(usize, char, &str)> {
    let mut j = s.len() - 1;
    let mut ch = 'X';
    for (i, c) in s.char_indices() {
        if (token == ';' && c == ';')
            || (token == '{' && c == '}')
            || (token == '<' && c == '>')
            || (token == '"' && c == '"')
        {
            j = i;
            ch = c;
            break;
        }
    }
    if j != s.len() - 1 {
        return Some((j, ch, &s[..j]));
    } else {
        None
    }
}

/// Main parser for C++ header files
///
/// This parser processes C++ header content and creates corresponding
/// Binary Ninja types including structs, classes, templates, and enums.
pub struct Parser<'a> {
    /// Binary Ninja binary view reference
    bv: &'a BinaryView,
    /// Current namespace stack for tracking nested namespaces
    namespace_stack: Vec<String>,
}

impl<'a> Parser<'a> {
    /// Creates a new Parser instance with the provided BinaryView
    ///
    /// # Arguments
    /// * `bv` - Binary Ninja binary view reference
    ///
    /// # Returns
    /// A new `Parser` instance
    pub fn new(bv: &'a BinaryView) -> Self {
        Self {
            bv,
            namespace_stack: Vec::new(),
        }
    }

    /// Parses C++ header content and imports types into Binary Ninja
    ///
    /// Processes the header content to extract and define structs, classes,
    /// templates, and enums in Binary Ninja's type system.
    ///
    /// # Arguments
    /// * `contents` - The C++ header file content as a string
    /// Gets the current namespace path as a vector
    fn get_current_namespace_path(&self) -> Vec<String> {
        self.namespace_stack.clone()
    }

    /// Enters a new namespace
    fn enter_namespace(&mut self, namespace_name: &str) {
        self.namespace_stack.push(namespace_name.to_string());
        log::info!("Entering namespace: {}", self.namespace_stack.join("::"));
    }

    /// Exits the current namespace
    fn exit_namespace(&mut self) {
        if let Some(namespace) = self.namespace_stack.pop() {
            log::info!("Exiting namespace: {}", namespace);
        }
    }

    pub fn parse(mut self, contents: &str) {
        let mut templates = Vec::<Template>::new();
        let mut idx = 0usize;
        loop {
            // find next ;, {, <, "
            if let Some((i, c, mut s)) = find_next_token(&contents[idx..]) {
                s = s.trim();
                idx += i + 1;
                log::info!("Handling line: {}, {}, {}", i, s, c);
                // base case
                if i == 0 || s.starts_with("//") {
                    continue;
                }
                // structure, template, class definition
                // get closing token and index
                if s.starts_with("#include") {
                    let (i2, _, mut s2) = find_closing_token(&contents[idx..], c)
                        .expect("Could not find closing token");
                    s2 = s2.trim();
                    // throw out include statements
                    assert!(c == '<' || c == '"');
                    log::info!("Skipping line: {} {}", s, s2);
                    idx += i2 + 1;
                    continue;
                }
                match c {
                    '}' => {
                        // Assume this is a namespace closing brace and exit current namespace
                        self.exit_namespace();
                    }
                    '<' => {
                        // template
                        if s.starts_with("template") {
                            let (i2, _, mut s2) = find_closing_token(&contents[idx..], c)
                                .expect("Could not find closing token");
                            s2 = s2.trim();
                            // template definition, store for later declarations
                            log::info!("Got template type: {}", s2);
                            let typenames = parse_template_definition(s2)
                                .expect(&format!("Could not parse template definitions {s2}"));
                            let (i3, c3, s3) = find_next_token(&contents[idx + i2 + 1..])
                                .expect("Could not find closing token for template definition");
                            assert!(c3 == '{');
                            log::info!("Got template name: {}", s3);
                            let (i4, _, body) =
                                find_closing_token(&contents[idx + i2 + i3 + 2..], c3)
                                    .expect("Could not find closing token");
                            log::info!("Got template definition: {}", body);
                            idx += i3 + i4 + 2;
                            let t = Template::new(
                                s3,
                                body,
                                typenames,
                                self.get_current_namespace_path(),
                            );
                            templates.push(t);
                            idx += i2 + 1;
                        } else {
                            // s looks like `struct_name<` or `typedef struct_name<`.
                            // Find closing `;` to get the contents within the angled brackets.
                            let (i2, _, mut s2) = find_closing_token(&contents[idx..], ';')
                                .expect("Could not find closing token for template instantiation");
                            if s.starts_with("typedef ") {
                                let mut typedef_string = s[8..].to_string();
                                typedef_string.push('<');
                                typedef_string.push_str(&s2);
                                log::info!("Got typedef string: {}", typedef_string);
                                let t = Typedef::new(
                                    &typedef_string,
                                    self.get_current_namespace_path(),
                                );
                                t.define(self.bv);
                            } else {
                                log::info!("Handling line: {} {}", s, s2);
                                if let Some(stripped) = s2.strip_suffix('>') {
                                    s2 = stripped;
                                }
                                log::info!("Got template instantiation: {}", s2);
                                let typenames = parse_template_instantiation(s2)
                                    .expect(&format!("Could not parse template definitions {s2}"));
                                // check for template named `s` to declare
                                let t = templates
                                    .iter()
                                    .find(|x| &x.name == s.trim())
                                    .expect(&format!("Could not find template {s} for definition"));
                                t.define(typenames, self.bv);
                            }
                            idx += i2 + 1;
                        }
                    }
                    ';' => {
                        if s.starts_with("typedef ") {
                            // typedef like `typedef struct_1 struct_2;`. Templated types
                            // match to `<` and are handled above.
                            let t = Typedef::new(&s[8..], self.get_current_namespace_path());
                            t.define(self.bv);
                        } else {
                            // forward declaration
                            let (i2, _, mut s2) = find_closing_token(&contents[idx..], c)
                                .expect("Could not find closing token");
                            s2 = s2.trim();
                            log::info!("Got forward declaration {}", s2);
                            idx += i2 + 1;
                        }
                    }
                    '{' => {
                        // namespace, class, or struct definition
                        if s.starts_with("namespace") {
                            // Extract namespace name
                            let namespace_name = s.strip_prefix("namespace").unwrap().trim();
                            if !namespace_name.is_empty() {
                                self.enter_namespace(namespace_name);
                                // Continue parsing inside the namespace
                            }
                        } else if s.starts_with("struct") {
                            let (i2, _, mut s2) = find_closing_token(&contents[idx..], c)
                                .expect("Could not find closing token");
                            s2 = s2.trim();
                            log::info!("Got struct {}: {}", s, s2);
                            let mut structure =
                                Structure::new(s, s2, self.bv, self.get_current_namespace_path());
                            structure.define(self.bv);
                            idx += i2 + 1;
                        } else if s.starts_with("class") {
                            let (i2, _, mut s2) = find_closing_token(&contents[idx..], c)
                                .expect("Could not find closing token");
                            s2 = s2.trim();
                            log::debug!("Got class {}: {}", s, s2);
                            let class =
                                Class::new(s, s2, self.bv, self.get_current_namespace_path());
                            class.define(self.bv);
                            idx += i2 + 1;
                        } else if s.starts_with("enum") {
                            let (i2, _, mut s2) = find_closing_token(&contents[idx..], c)
                                .expect("Could not find closing token");
                            s2 = s2.trim();

                            // Parse enum definition with optional size
                            let enum_with_size_regex =
                                Regex::new(r"enum\s+(\w+)\s*:\s*(\w+)").unwrap();
                            let enum_no_size_regex = Regex::new(r"enum\s+(\w+)").unwrap();

                            let (enum_name, size) =
                                if let Some(captures) = enum_with_size_regex.captures(s) {
                                    let enum_name = captures.get(1).unwrap().as_str();
                                    let size_str = captures.get(2).unwrap().as_str();

                                    let size = match size_str {
                                        "uint8_t" | "char" => 1,
                                        "uint16_t" | "short" => 2,
                                        "uint32_t" | "int" => 4,
                                        "uint64_t" | "long" => 8,
                                        _ => 4, // default to 4 bytes
                                    };

                                    (enum_name, size)
                                } else if let Some(captures) = enum_no_size_regex.captures(s) {
                                    let enum_name = captures.get(1).unwrap().as_str();
                                    (enum_name, 4) // default to uint32_t (4 bytes)
                                } else {
                                    panic!("Could not parse enum definition: {s}");
                                };

                            log::info!("Got enum {} with size {}", enum_name, size);
                            let enum_def =
                                Enum::new(enum_name, size, s2, self.get_current_namespace_path());
                            enum_def.define(self.bv);

                            idx += i2 + 1;
                        } else {
                            panic!("Could not handle definition: {s}");
                        }
                    }
                    _ => (),
                }
            } else {
                break;
            }
        }
    }
}

/// Binary Ninja command for importing C++ types from test.hpp
///
/// This command provides a user interface for triggering the C++ type import
/// functionality within Binary Ninja.
struct ImportCppTypesCommand;

impl Command for ImportCppTypesCommand {
    /// Executes the C++ type import command
    ///
    /// # Arguments
    /// * `view` - Binary Ninja binary view reference
    fn action(&self, view: &BinaryView) {
        info!("Importing C++ types from test.hpp");

        // Get the path to test.hpp (same as in the test)
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("../test.hpp");

        // Read the file contents
        match File::open(&path) {
            Ok(mut file) => {
                let mut contents = String::new();
                match file.read_to_string(&mut contents) {
                    Ok(_) => {
                        info!("Successfully read test.hpp, parsing C++ types...");
                        let parser = Parser::new(view);
                        parser.parse(&contents);
                        info!("C++ type import completed");
                    }
                    Err(e) => {
                        error!("Failed to read test.hpp: {}", e);
                    }
                }
            }
            Err(e) => {
                error!("Failed to open test.hpp: {}", e);
            }
        }
    }

    /// Determines if the command is valid for the current context
    ///
    /// # Arguments
    /// * `_view` - Binary Ninja binary view reference (unused)
    ///
    /// # Returns
    /// Always returns `true` as the command is always valid
    fn valid(&self, _view: &BinaryView) -> bool {
        // Command is always valid
        true
    }
}

/// Binary Ninja plugin initialization function
///
/// This function is called when the plugin is loaded by Binary Ninja.
/// It sets up logging and registers the C++ type import command.
///
/// # Returns
/// `true` if initialization succeeds, `false` otherwise
#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn CorePluginInit() -> bool {
    // Initialize logging
    Logger::new("C++ Type Importer")
        .with_level(LevelFilter::Info)
        .init();

    // Register the C++ Type Importer command
    register_command(
        "Import C++ Types",
        "Import C++ types from test.hpp into Binary Ninja's type system",
        ImportCppTypesCommand {},
    );

    info!("C++ Type Importer plugin initialized");

    true
}

#[cfg(test)]
mod tests {
    use binaryninja::headless::Session;
    use std::fs::File;
    use std::io::Read;
    use std::path::PathBuf;

    use crate::{
        get_type_width_by_name, is_primitive, parse_name, parse_template_definition,
        parse_template_instantiation, Class, Enum, Member, Parser, Structure, Template,
    };
    use binaryninja::binary_view::BinaryViewExt;

    #[test]
    fn test_parsing() {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let mut save_path = path.clone();
        path.push("../test.hpp");
        save_path.push("test.bndb");
        // get temp bv for arch
        let headless_session = Session::new().expect("Failed to initialize session");
        let bv = headless_session.load(&save_path).expect("Couldn't open bv");
        let mut file = File::open(&path).expect("Could not open test file");
        let mut contents = String::new();
        file.read_to_string(&mut contents)
            .expect("Could not read file contents");
        println!("File contents:\n{}", contents);
        let p = Parser {
            bv: bv.as_ref(),
            namespace_stack: vec![],
        };
        p.parse(&contents);
    }

    #[test]
    fn test_template_parsing() {
        let s = "typename X, class YZ";
        let typenames = parse_template_definition(s);
        assert_eq!(typenames.as_ref().unwrap().get(0), Some(&"X".to_string()));
        assert_eq!(typenames.as_ref().unwrap().get(1), Some(&"YZ".to_string()));
        let s = "T1, T2";
        let typenames = parse_template_definition(s);
        assert_eq!(typenames.as_ref().unwrap().get(0), Some(&"T1".to_string()));
        assert_eq!(typenames.as_ref().unwrap().get(1), Some(&"T2".to_string()));
    }

    #[test]
    fn test_template_instantiation_parsing() {
        let s = "uint32_t";
        let typenames = parse_template_instantiation(s);
        assert_eq!(
            typenames.as_ref().unwrap().get(0),
            Some(&"uint32_t".to_string())
        );
        let s = "uint32_t, void*";
        let typenames = parse_template_instantiation(s);
        assert_eq!(
            typenames.as_ref().unwrap().get(0),
            Some(&"uint32_t".to_string())
        );
        assert_eq!(
            typenames.as_ref().unwrap().get(1),
            Some(&"void*".to_string())
        );
        let s = "uint32_t, structure_name<void*, struct2_name>";
        let typenames = parse_template_instantiation(s);
        assert_eq!(
            typenames.as_ref().unwrap().get(0),
            Some(&"uint32_t".to_string())
        );
        assert_eq!(
            typenames.as_ref().unwrap().get(1),
            Some(&"structure_name<void*, struct2_name>".to_string())
        );
        let s = "structure_name<void*, struct2_name>, bool";
        let typenames = parse_template_instantiation(s);
        assert_eq!(
            typenames.as_ref().unwrap().get(0),
            Some(&"structure_name<void*, struct2_name>".to_string())
        );
        assert_eq!(
            typenames.as_ref().unwrap().get(1),
            Some(&"bool".to_string())
        );
        let s = "structure_name<void*, struct2_name<uint32_t, bool, void**>>, class_name<void*, uint64_t>";
        let typenames = parse_template_instantiation(s);
        assert_eq!(
            typenames.as_ref().unwrap().get(0),
            Some(&"structure_name<void*, struct2_name<uint32_t, bool, void**>>".to_string())
        );
        assert_eq!(
            typenames.as_ref().unwrap().get(1),
            Some(&"class_name<void*, uint64_t>".to_string())
        );
    }

    #[test]
    fn test_is_primitive() {
        // Test primitive types that should return Some
        assert!(is_primitive("char").is_some());
        assert!(is_primitive("unsigned char").is_some());
        assert!(is_primitive("int8_t").is_some());
        assert!(is_primitive("uint8_t").is_some());
        assert!(is_primitive("int16_t").is_some());
        assert!(is_primitive("uint16_t").is_some());
        assert!(is_primitive("int32_t").is_some());
        assert!(is_primitive("uint32_t").is_some());
        assert!(is_primitive("int").is_some());
        assert!(is_primitive("unsigned int").is_some());
        assert!(is_primitive("int64_t").is_some());
        assert!(is_primitive("uint64_t").is_some());
        assert!(is_primitive("bool").is_some());
        assert!(is_primitive("void").is_some());

        // Test non-primitive types that should return None
        assert!(is_primitive("MyStruct").is_none());
        assert!(is_primitive("std::string").is_none());
        assert!(is_primitive("vector<int>").is_none());
        assert!(is_primitive("custom_type").is_none());
        assert!(is_primitive("").is_none());
        assert!(is_primitive("float").is_none()); // not in the primitive map
        assert!(is_primitive("double").is_none()); // not in the primitive map
    }

    #[test]
    fn test_member_new_with_template_substitution() {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("test.bndb");
        let headless_session = Session::new().expect("Failed to initialize session");
        let bv = headless_session.load(&path).expect("Couldn't open bv");

        // First, define the basic types needed by templates
        // Define struct2_name as a basic struct
        let mut basic_struct =
            Structure::new("struct2_name", "int32_t x;", bv.as_ref(), Vec::new());
        basic_struct.define(bv.as_ref());
        assert_eq!(get_type_width_by_name(&"struct2_name", &bv), Some(4));

        // Define class_name as a basic struct
        let mut class_struct = Structure::new(
            "class_name",
            "int32_t field1;\nint32_t field2;",
            bv.as_ref(),
            Vec::new(),
        );
        class_struct.define(bv.as_ref());
        assert_eq!(get_type_width_by_name(&"class_name", &bv), Some(8));

        // Define a simple template structure
        let simple_template = Template::new(
            "structure_name",
            "T value;",
            vec!["T".to_string()],
            Vec::new(),
        );

        // Define template instantiations needed for the test
        // structure_name<uint32_t> and structure_name<void*>
        simple_template.define(vec!["uint32_t".to_string()], bv.as_ref());
        assert_eq!(
            get_type_width_by_name(&"structure_name<uint32_t>", &bv),
            Some(4)
        );
        simple_template.define(vec!["void*".to_string()], bv.as_ref());
        assert_eq!(
            get_type_width_by_name(&"structure_name<void*>", &bv),
            Some(8)
        );

        // Define a two-parameter template structure
        let two_param_template = Template::new(
            "structure_name_two",
            "T value1;\nU value2;",
            vec!["T".to_string(), "U".to_string()],
            Vec::new(),
        );

        // Define template instantiations needed for the test
        // structure_name_two<uint32_t, void*>
        two_param_template.define(
            vec!["uint32_t".to_string(), "void*".to_string()],
            bv.as_ref(),
        );
        assert_eq!(
            get_type_width_by_name(&"structure_name_two<uint32_t, void*>", &bv),
            Some(0x10)
        );

        // Define a more complex template for nested tests
        let nested_template = Template::new(
            "nested_struct",
            "T field1;\nU field2;",
            vec!["T".to_string(), "U".to_string()],
            Vec::new(),
        );
        // nested_struct<void*, struct2_name>
        nested_template.define(
            vec!["void*".to_string(), "struct2_name".to_string()],
            bv.as_ref(),
        );
        // TODO check packed
        assert_eq!(
            get_type_width_by_name(&"nested_struct<void*, struct2_name>", &bv),
            Some(0x10)
        );

        // Test templated function member
        let templated_function = Template::new(
            "function_struct",
            "T (*complex_func)(U param1, T* param2)\n",
            vec!["T".to_string(), "U".to_string()],
            Vec::new(),
        );
        templated_function.define(vec!["void".to_string(), "int32_t".to_string()], bv.as_ref());
        assert_eq!(
            get_type_width_by_name(&"function_struct<void, int32_t>", &bv),
            Some(8)
        );

        // Test simple template substitution
        // T -> uint32_t
        let template_members = vec!["T".to_string()];
        let template_defs = vec!["uint32_t".to_string()];

        let templated_member = Member::new(
            "structure_name<T> member_name",
            bv.as_ref(),
            Some(&template_members),
            Some(&template_defs),
            &Vec::new(),
        );

        let concrete_member = Member::new(
            "structure_name<uint32_t> member_name",
            bv.as_ref(),
            None,
            None,
            &Vec::new(),
        );

        // Both should have the same name and type
        assert_eq!(templated_member, concrete_member);

        // Test multiple template parameters: T, U -> uint32_t, void*
        let template_members = vec!["T".to_string(), "U".to_string()];
        let template_defs = vec!["uint32_t".to_string(), "void*".to_string()];

        let templated_member = Member::new(
            "structure_name_two<T, U> member_name",
            bv.as_ref(),
            Some(&template_members),
            Some(&template_defs),
            &Vec::new(),
        );

        let concrete_member = Member::new(
            "structure_name_two<uint32_t, void*> member_name",
            bv.as_ref(),
            None,
            None,
            &Vec::new(),
        );

        assert_eq!(templated_member, concrete_member);

        // Test template with pointer: T -> uint32_t for T* member
        let template_members = vec!["T".to_string()];
        let template_defs = vec!["uint32_t".to_string()];

        let templated_member = Member::new(
            "structure_name<T>* member_name",
            bv.as_ref(),
            Some(&template_members),
            Some(&template_defs),
            &Vec::new(),
        );

        let concrete_member = Member::new(
            "structure_name<uint32_t>* member_name",
            bv.as_ref(),
            None,
            None,
            &Vec::new(),
        );

        assert_eq!(templated_member, concrete_member);

        // Test templated function with single parameter: T -> uint32_t
        let template_members = vec!["T".to_string()];
        let template_defs = vec!["uint32_t".to_string()];

        let templated_function = Member::new(
            "T (*func_name)(T param)",
            bv.as_ref(),
            Some(&template_members),
            Some(&template_defs),
            &Vec::new(),
        );

        let concrete_function = Member::new(
            "uint32_t (*func_name)(uint32_t param)",
            bv.as_ref(),
            None,
            None,
            &Vec::new(),
        );

        assert_eq!(templated_function, concrete_function);

        // Test templated function with multiple parameters: T, U -> void, int32_t
        let template_members = vec!["T".to_string(), "U".to_string()];
        let template_defs = vec!["void".to_string(), "int32_t".to_string()];

        let templated_function = Member::new(
            "T (*complex_func)(U param1, T* param2)",
            bv.as_ref(),
            Some(&template_members),
            Some(&template_defs),
            &Vec::new(),
        );

        let concrete_function = Member::new(
            "void (*complex_func)(int32_t param1, void* param2)",
            bv.as_ref(),
            None,
            None,
            &Vec::new(),
        );

        assert_eq!(templated_function, concrete_function);
    }

    #[test]
    fn test_packed_structure() {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("test.bndb");
        let headless_session = Session::new().expect("Failed to initialize session");
        let bv = headless_session.load(&path).expect("Couldn't open bv");

        // Create a regular structure with padding
        let regular_struct_def = "struct RegularStruct";
        let regular_struct_body = "char a;\nint32_t b;\nchar c;";
        let mut regular_struct = Structure::new(
            regular_struct_def,
            regular_struct_body,
            bv.as_ref(),
            Vec::new(),
        );
        regular_struct.define(bv.as_ref());

        // Create a packed structure without padding
        let packed_struct_def = "struct __attribute__((packed)) PackedStruct";
        let packed_struct_body = "char a;\nint32_t b;\nchar c;";
        let mut packed_struct = Structure::new(
            packed_struct_def,
            packed_struct_body,
            bv.as_ref(),
            Vec::new(),
        );
        packed_struct.define(bv.as_ref());

        // Verify the packed flag was set correctly
        assert!(
            packed_struct.packed,
            "Packed struct should have packed flag set"
        );
        assert!(
            !regular_struct.packed,
            "Regular struct should not have packed flag set"
        );

        // Get the sizes of both structures
        let regular_size = get_type_width_by_name("RegularStruct", &bv)
            .expect("Could not get regular struct size");
        let packed_size =
            get_type_width_by_name("PackedStruct", &bv).expect("Could not get packed struct size");

        // The packed structure should be smaller than the regular structure
        // Regular: char(1) + 3 padding + int32_t(4) + char(1) + 3 padding = 12 bytes
        // Packed: char(1) + int32_t(4) + char(1) = 6 bytes
        assert_eq!(
            regular_size, 12,
            "Regular struct should be 12 bytes with padding"
        );
        assert_eq!(
            packed_size, 6,
            "Packed struct should be 6 bytes without padding"
        );

        println!("Regular struct size: {} bytes", regular_size);
        println!("Packed struct size: {} bytes", packed_size);
    }

    #[test]
    fn test_parse_name_with_packed_attribute() {
        // Test regular struct name parsing
        assert_eq!(parse_name("struct MyStruct"), Some("MyStruct".to_string()));

        // Test packed struct name parsing - attribute between struct and name
        assert_eq!(
            parse_name("struct __attribute__((packed)) MyPackedStruct"),
            Some("MyPackedStruct".to_string())
        );

        // Test class name parsing
        assert_eq!(parse_name("class MyClass"), Some("MyClass".to_string()));

        // Test enum name parsing
        assert_eq!(parse_name("enum MyEnum"), Some("MyEnum".to_string()));
    }

    #[test]
    fn test_namespace_parsing() {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("test.bndb");
        let headless_session = Session::new().expect("Failed to initialize session");
        let bv = headless_session.load(&path).expect("Couldn't open bv");

        // Test namespace stack functionality
        let mut parser = Parser::new(bv.as_ref());

        // Test entering and exiting namespaces
        parser.enter_namespace("QQQ");
        assert_eq!(parser.get_current_namespace_path(), vec!["QQQ".to_string()]);

        parser.enter_namespace("RRR");
        assert_eq!(
            parser.get_current_namespace_path(),
            vec!["QQQ".to_string(), "RRR".to_string()]
        );

        parser.exit_namespace();
        assert_eq!(parser.get_current_namespace_path(), vec!["QQQ".to_string()]);

        parser.exit_namespace();
        assert_eq!(parser.get_current_namespace_path(), Vec::<String>::new());
    }

    #[test]
    fn test_namespace_structure_definitions() {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("test.bndb");
        let headless_session = Session::new().expect("Failed to initialize session");
        let bv = headless_session.load(&path).expect("Couldn't open bv");

        // Define structures in different namespaces
        let mut global_struct = Structure::new(
            "struct aaa",
            "int32_t a;\nint32_t b;",
            bv.as_ref(),
            Vec::new(),
        );
        global_struct.define(bv.as_ref());

        let mut qqq_struct = Structure::new(
            "struct aaa",
            "uint32_t a;\nvoid* b;",
            bv.as_ref(),
            vec!["QQQ".to_string()],
        );
        qqq_struct.define(bv.as_ref());

        let mut rrr_struct = Structure::new(
            "struct aaa",
            "void* a;\nuint32_t b;\nuint64_t c;",
            bv.as_ref(),
            vec!["QQQ".to_string(), "RRR".to_string()],
        );
        rrr_struct.define(bv.as_ref());

        // Test get_full_name functionality
        assert_eq!(global_struct.get_full_name(), "aaa");
        assert_eq!(qqq_struct.get_full_name(), "QQQ::aaa");
        assert_eq!(rrr_struct.get_full_name(), "QQQ::RRR::aaa");

        // Verify types are defined in Binary Ninja with correct names
        assert!(bv.type_id_by_name("aaa").is_some());
        assert!(bv.type_id_by_name("QQQ::aaa").is_some());
        assert!(bv.type_id_by_name("QQQ::RRR::aaa").is_some());

        // Verify they have different sizes to confirm they're different types
        assert_eq!(get_type_width_by_name("aaa", &bv), Some(8)); // int32_t + int32_t
        assert_eq!(get_type_width_by_name("QQQ::aaa", &bv), Some(0x10)); // uint32_t + void* (with padding)
        assert_eq!(get_type_width_by_name("QQQ::RRR::aaa", &bv), Some(0x18)); // void* + uint32_t + uint64_t (with padding)
    }

    #[test]
    fn test_namespace_type_resolution() {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("test.bndb");
        let headless_session = Session::new().expect("Failed to initialize session");
        let bv = headless_session.load(&path).expect("Couldn't open bv");

        // Define types in different namespaces
        let mut global_struct =
            Structure::new("struct TestType", "int32_t x;", bv.as_ref(), Vec::new());
        global_struct.define(bv.as_ref());

        let mut ns_struct = Structure::new(
            "struct TestType",
            "uint32_t y;",
            bv.as_ref(),
            vec!["NS".to_string()],
        );
        ns_struct.define(bv.as_ref());

        // Test type resolution from different namespace contexts
        let global_context = Vec::new();
        let ns_context = vec!["NS".to_string()];

        // From global context, should find global type
        assert_eq!(
            Member::resolve_type_name("TestType", bv.as_ref(), &global_context),
            "TestType"
        );

        // From NS context, should find namespaced type first
        assert_eq!(
            Member::resolve_type_name("TestType", bv.as_ref(), &ns_context),
            "NS::TestType"
        );

        // Fully qualified names should resolve as-is
        assert_eq!(
            Member::resolve_type_name("NS::TestType", bv.as_ref(), &global_context),
            "NS::TestType"
        );
    }

    #[test]
    fn test_namespace_member_resolution() {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("test.bndb");
        let headless_session = Session::new().expect("Failed to initialize session");
        let bv = headless_session.load(&path).expect("Couldn't open bv");

        // Define a type in a namespace
        let mut ns_struct = Structure::new(
            "struct MyType",
            "int32_t value;",
            bv.as_ref(),
            vec!["TestNS".to_string()],
        );
        ns_struct.define(bv.as_ref());

        // Create a member that references this type from within the same namespace
        let member = Member::new(
            "MyType member_field",
            bv.as_ref(),
            None,
            None,
            &vec!["TestNS".to_string()],
        );

        // Should resolve to the namespaced type
        if let Member::Basic { typ, .. } = member {
            // The type should be resolved to the namespaced version
            assert!(typ.to_string().contains("TestNS::MyType"));
        } else {
            panic!("Expected basic member");
        }
    }

    #[test]
    fn test_namespace_enum_definitions() {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("test.bndb");
        let headless_session = Session::new().expect("Failed to initialize session");
        let bv = headless_session.load(&path).expect("Couldn't open bv");

        // Define enums in different namespaces
        let global_enum = Enum::new("GlobalEnum", 4, "VALUE1,\nVALUE2,\nVALUE3", Vec::new());
        global_enum.define(bv.as_ref());

        let ns_enum = Enum::new(
            "GlobalEnum",
            4,
            "NS_VALUE1,\nNS_VALUE2",
            vec!["MyNS".to_string()],
        );
        ns_enum.define(bv.as_ref());

        // Test get_full_name functionality for enums
        assert_eq!(global_enum.get_full_name(), "GlobalEnum");
        assert_eq!(ns_enum.get_full_name(), "MyNS::GlobalEnum");

        // Verify types are defined in Binary Ninja with correct names
        assert!(bv.type_id_by_name("GlobalEnum").is_some());
        assert!(bv.type_id_by_name("MyNS::GlobalEnum").is_some());
    }

    #[test]
    fn test_namespace_template_definitions() {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("test.bndb");
        let headless_session = Session::new().expect("Failed to initialize session");
        let bv = headless_session.load(&path).expect("Couldn't open bv");

        // Define templates in different namespaces
        let global_template =
            Template::new("Container", "T value;", vec!["T".to_string()], Vec::new());

        let ns_template = Template::new(
            "Container",
            "T data;\nint32_t size;",
            vec!["T".to_string()],
            vec!["Utils".to_string()],
        );

        // Test get_full_name functionality for templates
        assert_eq!(global_template.get_full_name(), "Container");
        assert_eq!(ns_template.get_full_name(), "Utils::Container");

        // Instantiate templates
        global_template.define(vec!["int32_t".to_string()], bv.as_ref());
        ns_template.define(vec!["int32_t".to_string()], bv.as_ref());

        // Verify instantiated types have correct names
        assert!(bv.type_id_by_name("Container<int32_t>").is_some());
        assert!(bv.type_id_by_name("Utils::Container<int32_t>").is_some());

        // They should have different sizes
        assert_eq!(get_type_width_by_name("Container<int32_t>", &bv), Some(4)); // Just T value
        assert_eq!(
            get_type_width_by_name("Utils::Container<int32_t>", &bv),
            Some(8)
        ); // T data + int32_t size
    }

    #[test]
    fn test_namespace_class_definitions() {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("test.bndb");
        let headless_session = Session::new().expect("Failed to initialize session");
        let bv = headless_session.load(&path).expect("Couldn't open bv");

        // Define classes in different namespaces
        let global_class = Class::new(
            "class MyClass",
            "MyClass();\n~MyClass();\n// ; end vtable\nint32_t value;",
            bv.as_ref(),
            Vec::new(),
        );
        global_class.define(bv.as_ref());

        let ns_class = Class::new(
            "class MyClass",
            "MyClass();\n~MyClass();\n// ; end vtable\nint64_t data;",
            bv.as_ref(),
            vec!["Services".to_string()],
        );
        ns_class.define(bv.as_ref());

        // Test get_full_name functionality for classes
        assert_eq!(global_class.get_full_name(), "MyClass");
        assert_eq!(ns_class.get_full_name(), "Services::MyClass");

        // Verify types are defined in Binary Ninja with correct names
        assert!(bv.type_id_by_name("MyClass").is_some());
        assert!(bv.type_id_by_name("Services::MyClass").is_some());
    }

    #[test]
    fn test_full_namespace_parsing() {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("test.bndb");
        let headless_session = Session::new().expect("Failed to initialize session");
        let bv = headless_session.load(&path).expect("Couldn't open bv");

        let namespace_code = r#"
            namespace QQQ {
                struct aaa {
                    uint32_t a;
                    void* b;
                };
                
                namespace RRR {
                    struct aaa {
                        void* a;
                        bool b;
                        uint64_t c;
                    };
                    
                    struct bbb {
                        aaa a;
                    };            

                    struct ccc {
                        QQQ::aaa a;
                    };
                }
            }
        "#;

        let parser = Parser::new(bv.as_ref());
        parser.parse(namespace_code);

        // Verify all types are defined with correct namespace prefixes
        assert!(bv.type_id_by_name("QQQ::aaa").is_some());
        assert!(bv.type_id_by_name("QQQ::RRR::aaa").is_some());
        assert!(bv.type_id_by_name("QQQ::RRR::bbb").is_some());

        // Verify they have the expected sizes
        assert_eq!(get_type_width_by_name("QQQ::aaa", &bv), Some(0x10)); // uint32_t + void*
        assert_eq!(get_type_width_by_name("QQQ::RRR::aaa", &bv), Some(0x18)); // void* + bool + padding + uint64_t
        assert_eq!(get_type_width_by_name("QQQ::RRR::bbb", &bv), Some(0x18)); // QQQ::RRR::aaa
        assert_eq!(get_type_width_by_name("QQQ::RRR::ccc", &bv), Some(0x10)); // QQQ::aaa
    }
}
