use binaryninja::architecture::CoreArchitecture;
use binaryninja::binary_view::BinaryViewExt;
use binaryninja::command::{register_command, Command};
// use binaryninja::logger::Logger;
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
    pub fn new(name: &str, size: u8, body: &str) -> Self {
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
                    dbg!(format!(
                        "Warning: Could not parse enum value '{value_str}', using {current_value}"
                    ));
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
        }
    }

    /// Defines the enum in Binary Ninja's type system
    ///
    /// # Arguments
    /// * `bv` - Binary Ninja binary view reference
    ///
    /// # Returns
    /// `true` if the enum was successfully defined
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
        bv.define_user_type(&self.name, &enum_type);
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
    pub fn new(def: &str, body: &str, typenames: Vec<String>) -> Self {
        let name = parse_name(def).expect(&format!("Could not parse name from {def}"));
        Self {
            name,
            typenames,
            body: body.to_string(),
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
            dbg!(&format!("Member {member}"));
            // Create a new member and provide the typenames to swap in case
            // the given member uses a typename
            members.push(Member::new(
                member,
                bv,
                Some(&self.typenames),
                Some(&typenames),
            ));
        }
        // `self.name` is simply the name of the templated structure. Add
        // `<typename1, typename2, ...>` to distinguish this particular
        // instantiation.
        let mut name = self.name.clone();
        name.push('<');
        name.push_str(&typenames.join(", "));
        name.push('>');
        Structure::new_from_members(name, members, 0).define(bv);
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
    pub fn new<'b>(def: &str, body: &str, bv: &'a BinaryView) -> Self {
        let name = parse_name(def).expect(&format!("Could not parse definition {def} for name"));
        let mut members = vec![];
        for member in body.lines() {
            // Create a new member with no templated fields
            members.push(Member::new(member, bv, None, None));
        }

        Self {
            name,
            members,
            offset: 0,
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
    pub fn new_from_members(name: String, members: Vec<Member>, offset: u16) -> Self {
        Self {
            name,
            members,
            offset,
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
        for m in self.members.iter_mut() {
            match m {
                Member::Basic {
                    name,
                    typ,
                    comments,
                } => {
                    // Simply append basic members
                    dbg!(&format!("Adding member: {name}"));
                    builder.append(
                        typ.as_ref(),
                        &name.clone(),
                        MemberAccess::PublicAccess,
                        MemberScope::NoScope,
                    );
                }
                Member::Function { name, ret, args } => {
                    // Create a function type and pointer to that function
                    dbg!(&format!("Adding function: {name}"));
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
                    builder.append(
                        func.as_ref(),
                        &name.clone(),
                        MemberAccess::PublicAccess,
                        MemberScope::NoScope,
                    );
                }
            }
        }
        let s = Type::structure(&builder.finalize());
        dbg!(format!("Defining structure {}", self.name));
        bv.define_user_type(&self.name, &s);
        true
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
    pub fn new(def: &str, body: &str, bv: &'a BinaryView) -> Self {
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
        bv.define_user_type(&name, &forward_decl);

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
                member_variables.push((Member::new(&clean_line, bv, None, None), override_info));
            }
        }

        Self {
            name,
            vtable_methods,
            member_variables,
            base_classes,
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
            return Member::new(&method_signature, bv, None, None).into();
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

                return Member::new(&method_signature, bv, None, None).into();
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

            Member::new(&method_signature, bv, None, None).into()
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
            if let Member::Function { name, ret, args } = method {
                let mut params = vec![];
                for (arg_name, arg_type) in args {
                    params.push(FunctionParameter::new(
                        arg_type.clone(),
                        arg_name.clone(),
                        None,
                    ));
                }
                let func = Type::function(ret.as_ref(), params, false);
                let func_ptr = Type::pointer(
                    &bv.default_arch().expect("Could not find default arch"),
                    func.as_ref(),
                );

                if let Some(override_str) = override_info {
                    // Check if this override targets the current base class
                    dbg!(&format!("Handling override: {override_str}"));
                    // TODO what about multiple levels of inheritance?
                    if Self::is_override_for_base_class(override_str, base_class) {
                        // Parse the override information to find the target method
                        if let Some(offset) =
                            Self::parse_override_offset(override_str, base_class, bv)
                        {
                            // Override at specific offset
                            vtable_builder.insert(
                                func_ptr.as_ref(),
                                name,
                                offset,
                                true, // overwrite existing
                                MemberAccess::PublicAccess,
                                MemberScope::NoScope,
                            );
                        }
                    }
                } else if process_non_overriding {
                    // No override, add to vtable only if processing non-overriding methods
                    vtable_builder.append(
                        func_ptr.as_ref(),
                        name,
                        MemberAccess::PublicAccess,
                        MemberScope::NoScope,
                    );
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
                                dbg!(&format!(
                                    "Found override for '{cleaned_override}' in {base_class}",
                                ));
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
                                    dbg!(&format!(
                                        "Found member override '{cleaned_override}' in base class '{base_class}'",
                                    ));
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
                if let Member::Function { name, ret, args } = method {
                    // TODO migrate this logic to Member::define
                    let mut params = vec![];
                    for (arg_name, arg_type) in args {
                        params.push(FunctionParameter::new(
                            arg_type.clone(),
                            arg_name.clone(),
                            None,
                        ));
                    }
                    let func = Type::function(ret.as_ref(), params, false);
                    let func_ptr = Type::pointer(
                        &bv.default_arch().expect("Could not find default arch"),
                        func.as_ref(),
                    );
                    vtable_builder.append(
                        func_ptr.as_ref(),
                        name,
                        MemberAccess::PublicAccess,
                        MemberScope::NoScope,
                    );
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

                    // Get the width of the base vtable. This works with `NamedTypedReference`
                    // because the underlying type is a pointer. BEWARE, this does not work
                    // if it was a defined structure.
                    if let Some(base_vtable_type) = bv.type_by_id(&base_vtable_type_id) {
                        base_vtable_width = base_vtable_type.width();
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
                let base_width = if let Some(base_type) = bv.type_by_id(&base_type_id) {
                    base_type.width()
                } else {
                    0
                };

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
                    if let Some(base_type_id) = bv.type_id_by_name(base_class) {
                        if let Some(base_type) = bv.type_by_id(&base_type_id) {
                            current_offset += base_type.width();
                        }
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
                        if let Some(prev_base_type_id) = bv.type_id_by_name(base) {
                            if let Some(prev_base_type) = bv.type_by_id(&prev_base_type_id) {
                                actual_offset += prev_base_type.width();
                            }
                        }
                    }

                    // Insert the overriding member at the calculated offset
                    // TODO move this to Member::define
                    match member {
                        Member::Basic {
                            name,
                            typ,
                            comments: _,
                        } => {
                            class_builder.insert(
                                typ.as_ref(),
                                name,
                                actual_offset,
                                true, // overwrite existing
                                MemberAccess::PublicAccess,
                                MemberScope::NoScope,
                            );
                        }
                        Member::Function { name, ret, args } => {
                            let mut params = vec![];
                            for (arg_name, arg_type) in args {
                                params.push(FunctionParameter::new(
                                    arg_type.clone(),
                                    arg_name.clone(),
                                    None,
                                ));
                            }
                            let func = Type::function(ret.as_ref(), params, false);
                            let func_ptr = Type::pointer(
                                &bv.default_arch().expect("Could not find default arch"),
                                func.as_ref(),
                            );
                            class_builder.insert(
                                func_ptr.as_ref(),
                                name,
                                actual_offset,
                                true, // overwrite existing
                                MemberAccess::PublicAccess,
                                MemberScope::NoScope,
                            );
                        }
                        _ => (),
                    }
                    continue;
                }
            }

            // No override, append normally
            // TODO move this to Member::define
            match member {
                Member::Basic {
                    name,
                    typ,
                    comments: _,
                } => {
                    class_builder.append(
                        typ.as_ref(),
                        name,
                        MemberAccess::PublicAccess,
                        MemberScope::NoScope,
                    );
                }
                Member::Function { name, ret, args } => {
                    // Handle function pointers in member variables
                    let mut params = vec![];
                    for (arg_name, arg_type) in args {
                        params.push(FunctionParameter::new(
                            arg_type.clone(),
                            arg_name.clone(),
                            None,
                        ));
                    }
                    let func = Type::function(ret.as_ref(), params, false);
                    let func_ptr = Type::pointer(
                        &bv.default_arch().expect("Could not find default arch"),
                        func.as_ref(),
                    );
                    class_builder.append(
                        func_ptr.as_ref(),
                        name,
                        MemberAccess::PublicAccess,
                        MemberScope::NoScope,
                    );
                }
            }
        }

        // Define the class structure
        let class_structure = Type::structure(&class_builder.finalize());
        bv.define_user_type(&self.name, &class_structure);

        true
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

impl Member {
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
        dbg!(&format!("Defining type {t}"));
        let mut typ = if let Some(tt) = is_primitive(t) {
            tt
        } else {
            // Try to find an existing type ID
            if let Some(type_id) = bv.type_id_by_name(t) {
                let named_ref = NamedTypeReference::new_with_id(
                    NamedTypeReferenceClass::StructNamedTypeClass,
                    &type_id,
                    t,
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
                panic!("Could not find type: {}", t);
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

    /// Creates a new member from its definition string
    ///
    /// # Arguments
    /// * `def` - The member definition string, e.g., `type_name** type`
    /// * `bv` - Binary Ninja binary view reference
    /// * `template_members` - Optional template parameter names
    /// * `template_defs` - Optional template parameter definitions
    ///
    /// # Returns
    /// A new `Member` instance
    fn new(
        def: &str,
        bv: &BinaryView,
        template_members: Option<&Vec<String>>,
        template_defs: Option<&Vec<String>>,
    ) -> Self {
        let (typ, name, depth) =
            parse_member_definition(def).expect("Could not parse member definition");
        dbg!(format!(
            "Got member definition type={typ}, name={name}, depth={depth}"
        ));
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
                dbg!(&format!("Instantitated templated type {def}"));
            }
            // Try to match function definition: `return_type (*name)(args)`
            let func_regex = Regex::new(r"(.*) \(\*(.*)\)\((.*)\)").unwrap();
            if let Some(captures) = func_regex.captures(&def) {
                let return_type = captures.get(1).unwrap().as_str().trim();
                let name = captures.get(2).unwrap().as_str().trim();
                let args = captures.get(3).unwrap().as_str().trim();

                dbg!(format!(
                    "Got function definition: return_type={}, name={}, args={:#}",
                    return_type, name, args
                ));
                let (return_type, _, depth) = parse_member_definition(return_type)
                    .expect("Could not parse function member return type");
                let args = parse_template_instantiation(args)
                    .expect("Could not parse function member args");
                let mut defined_args = vec![];
                for a in args {
                    let (typ, name, depth) = parse_member_definition(&a)
                        .expect("Could not parse argument to function definition");
                    defined_args.push((name, Self::define_type(&typ, depth, bv)));
                }

                return Member::Function {
                    name: name.to_string(),
                    ret: Self::define_type(&return_type, depth, bv),
                    args: defined_args,
                };
            } else {
                // Not a function definition
                let (typ, name, depth) =
                    parse_member_definition(&def).expect(&format!("Could not parse {def}"));
                let typ = Self::define_type(&typ, depth, bv);
                return Member::Basic {
                    name,
                    typ,
                    comments: vec![],
                };
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
/// Handles nested template arguments with proper bracket matching.
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
/// Filters out keywords like 'typename' and 'class'.
///
/// # Arguments
/// * `s` - The template definition string (contents between angle brackets), like
/// `typename1, struct_two<typename2, typename3>`.
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
/// by stripping the preceding type identifier.
///
/// # Arguments
/// * `def` - The definition string, e.g., `struct MyStruct`
///
/// # Returns
/// `Some(String)` with the parsed name, `None` if parsing fails
fn parse_name(def: &str) -> Option<String> {
    let mut s = def.trim();
    if s.starts_with("struct ") {
        s = s.strip_prefix("struct ")?;
    } else if s.starts_with("class ") {
        s = s.strip_prefix("class ")?;
    } else if s.starts_with("enum ") {
        s = s.strip_prefix("enum ")?;
    } else if s.starts_with("template ") {
        s.strip_prefix("template ")?;
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
        if c == '{' || c == ';' || c == '<' || c == '"' {
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
        Self { bv }
    }

    /// Parses C++ header content and imports types into Binary Ninja
    ///
    /// Processes the header content to extract and define structs, classes,
    /// templates, and enums in Binary Ninja's type system.
    ///
    /// # Arguments
    /// * `contents` - The C++ header file content as a string
    pub fn parse(self, contents: &str) {
        let mut templates = Vec::<Template>::new();
        let mut idx = 0usize;
        loop {
            // find next ;, {, <, "
            if let Some((i, c, mut s)) = find_next_token(&contents[idx..]) {
                s = s.trim();
                idx += i + 1;
                dbg!(format!("{i}, {s}, {c}"));
                // base case
                if i == 0 {
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
                    dbg!(format!("Skipping {s} {s2}"));
                    idx += i2 + 1;
                    continue;
                }
                match c {
                    '<' => {
                        // template
                        if s.starts_with("template") {
                            let (i2, _, mut s2) = find_closing_token(&contents[idx..], c)
                                .expect("Could not find closing token");
                            s2 = s2.trim();
                            // template definition, store for later declarations
                            dbg!(format!("Got template type {s2}"));
                            let typenames = parse_template_definition(s2)
                                .expect(&format!("Could not parse template definitions {s2}"));
                            let (i3, c3, mut s3) = find_next_token(&contents[idx + i2 + 1..])
                                .expect("Could not find closing token for template definition");
                            assert!(c3 == '{');
                            dbg!(format!("{typenames:?}"));
                            dbg!(format!("Got template name {s3}"));
                            let (i4, _, mut body) =
                                find_closing_token(&contents[idx + i2 + i3 + 2..], c3)
                                    .expect("Could not find closing token");
                            dbg!(format!("Got template definition {body}"));
                            idx += i3 + i4 + 2;
                            let t = Template::new(s3, body, typenames);
                            println!("{:?}", t);
                            templates.push(t);
                            idx += i2 + 1;
                        } else {
                            // s looks like `structname<`. Find closing `;` to get the contents
                            // within the angled brackets.
                            let (i2, _, mut s2) = find_closing_token(&contents[idx..], ';')
                                .expect("Could not find closing token for template instantiation");
                            println!("{s} {s2}");
                            if let Some(stripped) = s2.strip_suffix('>') {
                                s2 = stripped;
                            }
                            dbg!(format!("Got template instantiation {s2}"));
                            let typenames = parse_template_instantiation(s2)
                                .expect(&format!("Could not parse template definitions {s2}"));
                            dbg!(format!("{typenames:?}"));
                            // check for template named `s` to declare
                            let t = templates
                                .iter()
                                .find(|x| &x.name == s.trim())
                                .expect(&format!("Could not find template {s} for definition"));
                            t.define(typenames, self.bv);
                            println!("{idx}");
                            idx += i2 + 1;
                            println!("{idx}");
                        }
                    }
                    ';' => {
                        // forward declaration
                        let (i2, _, mut s2) = find_closing_token(&contents[idx..], c)
                            .expect("Could not find closing token");
                        s2 = s2.trim();
                        dbg!(format!("Got forward declaration {s2}"));
                        idx += i2 + 1;
                    }
                    '{' => {
                        // class or struct definition
                        if s.starts_with("struct") {
                            let (i2, _, mut s2) = find_closing_token(&contents[idx..], c)
                                .expect("Could not find closing token");
                            s2 = s2.trim();
                            dbg!("Got struct");
                            dbg!(format!("{s}: {s2}"));
                            let mut structure = Structure::new(s, s2, self.bv);
                            dbg!(format!("{structure:?}",));
                            structure.define(self.bv);
                            idx += i2 + 1;

                            // for l in s2.lines() {
                            //     dbg!(format!("Got Member {:#?}", Member::parse(l, vec![])));
                            // }
                        } else if s.starts_with("class") {
                            let (i2, _, mut s2) = find_closing_token(&contents[idx..], c)
                                .expect("Could not find closing token");
                            s2 = s2.trim();
                            dbg!("Got class");
                            dbg!(format!("{s}: {s2}"));
                            let class = Class::new(s, s2, self.bv);
                            dbg!(format!("{class:?}"));
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

                            dbg!(format!("Got enum {enum_name} with size {size}"));
                            let enum_def = Enum::new(enum_name, size, s2);
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
    log::set_max_level(LevelFilter::Debug);

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

    // fn get_binary_view() -> Ref<BinaryView> {

    // println!("Filename:  `{}`", bv.file().filename());
    // println!("File size: `{:#x}`", bv.len());
    // println!("Function count: {}", bv.functions().len());
    // bv
    // }

    use crate::{parse_template_definition, Parser};

    #[test]
    fn test_parsing() {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let mut save_path = path.clone();
        path.push("../test.hpp");
        save_path.push("test.bndb");
        // get temp bv for arch
        println!("{save_path:?}");
        let headless_session = Session::new().expect("Failed to initialize session");
        let bv = headless_session.load(&save_path).expect("Couldn't open bv");
        let mut file = File::open(&path).expect("Could not open test file");
        let mut contents = String::new();
        file.read_to_string(&mut contents)
            .expect("Could not read file contents");
        println!("File contents:\n{}", contents);
        let p = Parser { bv: bv.as_ref() };
        p.parse(&contents);
        // println!("Tyring to save");
        // assert!(bv.save_to_path(&save_path));
    }

    #[test]
    fn test_template_parsing() {
        let s = "typename X, class YZ";
        let typenames = parse_template_definition(s);
        assert_eq!(typenames.as_ref().unwrap().get(0), Some(&"X".to_string()));
        assert_eq!(typenames.as_ref().unwrap().get(1), Some(&"YZ".to_string()));
    }
}
