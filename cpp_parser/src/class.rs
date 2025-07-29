use binaryninja::binary_view::{BinaryView, BinaryViewExt};
use binaryninja::types::{
    BaseStructure, MemberAccess, MemberScope, NamedTypeReference, NamedTypeReferenceClass,
    StructureBuilder, Type,
};
use regex::Regex;

use crate::{get_type_width_by_name, parse_name, parse_template_instantiation, Member};

/// Represents a C++ class with virtual table, members, and inheritance
///
/// This structure supports C++ class features including virtual methods,
/// member variables, and inheritance from base classes.
#[derive(Debug)]
pub struct Class {
    /// The name of the class
    pub name: String,
    /// Virtual table methods with optional override information
    pub vtable_methods: Vec<(Member, Option<String>)>,
    /// Member variables with optional override information
    pub member_variables: Vec<(Member, Option<String>)>,
    /// List of base class names for inheritance
    pub base_classes: Vec<String>,
    /// Namespace path for this class
    pub namespace_path: Vec<String>,
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
    pub fn new(
        def: &str,
        body: &str,
        bv: &'a BinaryView,
        namespace_path: Vec<String>,
    ) -> Result<Self, String> {
        // Parse class name and inheritance with regex
        // TODO parse inherited classes differently, inherited classes could be templated
        let class_regex = Regex::new(r"class\s+(\w+)(?:\s*:\s*(.+))?").unwrap();
        let (name, base_classes) = if let Some(captures) = class_regex.captures(def) {
            if let Some(class_name) = captures.get(1) {
                let class_name = class_name.as_str().to_string();
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
                log::error!("Could not find class name in {def}");
                ("".to_string(), vec![])
            }
        } else {
            // Fallback to existing `parse_name` logic
            if let Some(name) = parse_name(def) {
                (name, Vec::new())
            } else {
                log::error!("Could not parse class name from {def}");
                ("".to_string(), Vec::new())
            }
        };

        // Forward-declare the class as a structure so it can be referenced in constructor signatures
        let forward_decl = Type::structure(&StructureBuilder::new().finalize());
        let full_name = if namespace_path.is_empty() {
            name.clone()
        } else {
            format!("{}::{}", namespace_path.join("::"), name)
        };
        log::info!(
            "Forward declaring class {} with current namespace(s): {:?}",
            name,
            namespace_path
        );
        bv.define_user_type(&full_name, &forward_decl);

        let mut vtable_methods = Vec::new();
        let mut member_variables = Vec::new();
        let mut in_vtable = true;

        // Body contains a list of vtable methods (including any overrides) followed by a single
        // line `// ; end vtable` denoting the end of the vtable and start of members (including
        // overrides).
        for line in body.lines() {
            let line = line.trim();
            // Check for end of vtable marker
            if line.contains("// ; end vtable") {
                in_vtable = false;
                continue;
            }

            if line.is_empty() || line.starts_with("//") {
                continue;
            }

            if in_vtable {
                // Parse vtable method
                let (method, override_info) =
                    Self::parse_vtable_method(line, bv, &name, &namespace_path)?;
                vtable_methods.push((method, override_info));
            } else {
                // Parse member variable
                // TODO include offset information in this
                let override_regex = Regex::new(r"//\s*;\s*override\s+(.+?);").unwrap();
                let override_info = if let Some(captures) = override_regex.captures(line) {
                    if let Some(cap) = captures.get(1) {
                        Some(cap.as_str().to_string())
                    } else {
                        log::warn!("Could not find first regex capture group for override");
                        None
                    }
                } else {
                    None
                };

                // Remove override comment from the line
                let clean_line = override_regex.replace(line, "").trim().to_string();
                member_variables.push((
                    Member::new(&clean_line, bv, None, None, &namespace_path)?,
                    override_info,
                ));
            }
        }

        Ok(Self {
            name,
            vtable_methods,
            member_variables,
            base_classes,
            namespace_path,
        })
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
        namespace_path: &Vec<String>,
    ) -> Result<(Member, Option<String>), String> {
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
        let member =
            Self::parse_method_signature(&clean_line, bv, this_offset, class_name, namespace_path)?;
        Ok((member, override_info))
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
        namespace_path: &Vec<String>,
    ) -> Result<Member, String> {
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
            return Ok(Member::new(
                &method_signature,
                bv,
                None,
                None,
                &namespace_path,
            )?);
        }

        // Check for constructor: ClassName(...)
        if let Some(paren_pos) = line.find('(') {
            let potential_constructor = line[..paren_pos].trim();
            if potential_constructor == class_name {
                let params_str = &line[paren_pos + 1..line.rfind(')').unwrap_or(line.len())];

                // Parse all parameters first
                let mut all_params = if params_str.is_empty() {
                    Vec::new()
                } else if let Some(params) = parse_template_instantiation(params_str)? {
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

                return Member::new(&method_signature, bv, None, None, namespace_path).into();
            }
        }

        // For regular methods, inject `*this` pointer at the specified offset
        if let Some(paren_pos) = line.find('(') {
            let before_paren = &line[..paren_pos];
            let params_str = &line[paren_pos + 1..line.rfind(')').unwrap_or(line.len())];

            // Parse all parameters first
            let mut all_params = if params_str.is_empty() {
                Vec::new()
            } else if let Some(params) = parse_template_instantiation(params_str)? {
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

            Member::new(&method_signature, bv, None, None, &namespace_path).into()
        } else {
            Err(format!("Could not parse method signature {line}"))
        }
    }

    /// Processes virtual table methods for a specific base class
    ///
    /// # Arguments
    /// * `vtable_methods` - List of virtual table methods
    /// * `vtable_builder` - Structure builder for the virtual table
    /// * `base_class` - The base class vtable name (e.g., `HHH_vtable_III` or `JJJ_vtable`)
    /// * `process_non_overriding` - Whether to process non-overriding methods
    /// * `bv` - Binary Ninja binary view reference
    fn process_vtable_methods_for_base(
        &mut self,
        vtable_builder: &mut StructureBuilder,
        base_class_vtable_name: &str,
        process_non_overriding: bool,
        bv: &'a BinaryView,
    ) -> Result<(), String> {
        let mut remaining: Vec<(Member, Option<String>)> = vec![];
        for (method, override_info) in &self.vtable_methods {
            if let Member::Function { .. } = method {
                if let Some(override_str) = override_info {
                    // Check if this override targets the current base class
                    log::info!("Checking override: {}", override_str);
                    if override_str.contains(base_class_vtable_name) {
                        // Parse the override information to find the target method
                        if let Some(offset) =
                            Self::parse_override_offset(&override_str, base_class_vtable_name, bv)
                        {
                            // Override at specific offset
                            method.define(Some(offset), vtable_builder, bv)?;
                            continue;
                        }
                    }
                    remaining.push((method.clone(), Some(override_str.clone())));
                } else if process_non_overriding {
                    // No override, add to vtable only if processing non-overriding methods
                    method.define(None, vtable_builder, bv)?;
                } else {
                    remaining.push((method.clone(), override_info.clone()));
                }
            }
        }

        self.vtable_methods = remaining;

        // If the vtable inherits from another vtable, try seraching downward to
        // see if any of the base classes match. E.g., if D : C : B and C does not
        // override a function from B, it will still have `B::function_name` in its
        // vtable. Therefore, need to search B's vtable after trying C's.
        if let Some(base_vtable_type_id) = bv.type_id_by_name(base_class_vtable_name) {
            if let Some(base_vtable_type) = bv.type_by_id(&base_vtable_type_id) {
                if let Some(s) = base_vtable_type.get_structure() {
                    // Vtables should only inherit from one structure
                    if let Some(b) = s.base_structures().first() {
                        self.process_vtable_methods_for_base(
                            vtable_builder,
                            &b.ty.name().to_string(),
                            false, // Already added to the structure builder, don't add again
                            bv,
                        )?;
                    }
                }
            }
        }
        Ok(())
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
        base_vtable_name: &str,
        bv: &'a BinaryView,
    ) -> Option<u64> {
        // Parse override string like `void (* <base_vtable_name>::fn)(struct base* this);`
        // Find the vtable type and look for the method
        if let Some(base_vtable_type_id) = bv.type_id_by_name(base_vtable_name) {
            if let Some(base_vtable_type) = bv.type_by_id(&base_vtable_type_id) {
                // Get the structure members to find the method offset
                if let Some(structure) = base_vtable_type.get_structure() {
                    // Parse the method name from the override string. `cleaned_override`
                    // should look like `return_type (* fn)(args, ...)`
                    if let Some(cleaned_override) =
                        Self::extract_method_name_from_override(override_str)
                    {
                        // Find the method in the base vtable structure by reconstructing the signature
                        for member in structure.members().iter() {
                            // Get member type contents, which does not include the function name,
                            // just the return value, `(*)`, and arguments
                            let mut contents = member.ty.contents.to_string();

                            // Find `(*)` and insert the member name after the `*`
                            if let Some(star_pos) = contents.find("(*)") {
                                contents.insert_str(star_pos + 2, &format!(" {}", member.name));
                            }

                            if cleaned_override == contents {
                                log::info!(
                                    "Found override for '{}' in {} at offset 0x{:x}",
                                    cleaned_override,
                                    base_vtable_name,
                                    member.offset
                                );
                                return Some(member.offset);
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
    /// `void (* HHH_vtable_III::fn)(struct base* this)` or
    /// `void (* JJJ_vtable::fn)(struct base* this)`
    ///
    /// # Returns
    /// `Some(String)` with the cleaned method name (e.g., `void (* fn)(struct base* this)`),
    /// `None` if parsing fails
    fn extract_method_name_from_override(override_str: &str) -> Option<String> {
        // Parse override string like `void (* base_vtable::fn)(struct base* this);`
        // Remove the inherited `base_vtable::` prefix while keeping the rest
        let regex = Regex::new(r"\w+_vtable(\w+)?::").unwrap();
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
    /// * `base_classes` - List of base class names and their respective offsets
    /// * `bv` - Binary Ninja binary view reference
    ///
    /// # Returns
    /// `Some((String, u64))` with base class name and offset if found
    fn parse_member_override_offset(
        override_str: &str,
        base_classes: &[(String, u64)],
        bv: &'a BinaryView,
    ) -> Option<(String, u64)> {
        // Try to find the member in each base class
        for (base_class, base_offset) in base_classes {
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
                                        "Found member override '{}' in base class '{}' at offset {}+{base_offset}",
                                        cleaned_override,
                                        base_class,
                                        member.offset
                                    );
                                    return Some((base_class.clone(), member.offset + base_offset));
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Recursively collects all base classes for this class, including indirect inheritance
    ///
    /// # Arguments
    /// * `bv` - Binary Ninja binary view reference
    ///
    /// # Returns
    /// A vector of tuples (base_class_name, offset) for all base classes
    fn collect_all_base_classes(&self, bv: &'a BinaryView) -> Vec<(String, u64)> {
        let mut all_bases = Vec::new();
        let mut visited = std::collections::HashSet::new();

        // Depth-first search to collect all base classes with their offsets
        fn collect_recursive(
            override_vtable: bool,
            base_class: &str,
            current_offset: u64,
            all_bases: &mut Vec<(String, u64)>,
            visited: &mut std::collections::HashSet<String>,
            bv: &BinaryView,
        ) -> u64 {
            if visited.contains(base_class) {
                return current_offset;
            }

            visited.insert(base_class.to_string());
            // Used to differentiate vtable overwrites. E.g, consider
            // A : B, C and B : D, E and C : F, G, H. A should overwrite
            // vtables for B and C (top level, which override D and F),
            // B's overriden vtable for E and C's overriden vtable for G.
            // We need to keep track of all inherited base classes for overriding
            // functionality not touched in the middle classes (B, C), and thus
            // need to track that A still inherits members and potentially vtable
            // methods from D, E, F, G, and H. However, only the top level vtables
            // (B and C) and the `1..` inherited base class vtables (E, G, H) are
            // overridden. The first inherited class in each top-level class (D, F)
            // are stored with offset `u64::MAX` to indicate A does not override
            // their vtables.
            // if override_vtable {
            // } else {
            //     all_bases.push((base_class.to_string(), std::u64::MAX));
            // }
            all_bases.push((base_class.to_string(), current_offset));
            let mut updated_offset = current_offset;

            // Look up the base class type in Binary Ninja
            if let Some(base_type_id) = bv.type_id_by_name(base_class) {
                if let Some(base_type) = bv.type_by_id(&base_type_id) {
                    if let Some(base_structure) = base_type.get_structure() {
                        // Recursively collect base structures of this base class
                        for (i, base_struct) in base_structure.base_structures().iter().enumerate()
                        {
                            let override_vtable = if i != 0 { true } else { false };
                            let indirect_base_name = base_struct.ty.name().to_string();
                            updated_offset = collect_recursive(
                                override_vtable,
                                &indirect_base_name,
                                updated_offset,
                                all_bases,
                                visited,
                                bv,
                            );
                        }
                        // Increment offset by the width of this base class if this
                        // child has no children.
                        if base_structure.base_structures().is_empty() {
                            if let Some(width) = get_type_width_by_name(&base_class, bv) {
                                return current_offset + width;
                            }
                        }
                    }
                }
            }
            updated_offset
        }

        // Start DFS from each direct base class
        let mut current_offset = 0u64;
        for base_class in &self.base_classes {
            collect_recursive(
                true,
                base_class,
                current_offset,
                &mut all_bases,
                &mut visited,
                bv,
            );

            // Increment offset by the width of this direct base class
            if let Some(width) = get_type_width_by_name(base_class, bv) {
                current_offset += width;
            }
        }

        log::info!("Found all bases {:#x?}", all_bases);
        all_bases
    }

    /// Defines the class and its virtual tables in Binary Ninja's type system
    ///
    /// # Arguments
    /// * `bv` - Binary Ninja binary view reference
    ///
    /// # Returns
    /// `true` if the class was successfully defined
    pub fn define(&mut self, bv: &'a BinaryView) -> Result<(), String> {
        // Create separate vtables for overriding each base class's
        // Tuple of (new_vtable_name, inherited_vtable_name, offset)
        let mut vtable_names = Vec::new();

        if self.base_classes.is_empty() {
            // No inheritance - create regular vtable
            let vtable_name = format!("{}_vtable", self.get_full_name());
            let mut vtable_builder = StructureBuilder::new();

            for (method, _) in &self.vtable_methods {
                if let Member::Function { .. } = method {
                    method.define(None, &mut vtable_builder, bv)?;
                }
            }

            self.vtable_methods = vec![];

            let vtable_structure = Type::structure(&vtable_builder.finalize());
            bv.define_user_type(&vtable_name, &vtable_structure);
            vtable_names.push((vtable_name, 0));
        } else {
            // Has inheritance - collect all non-top-level base classes. Top-level
            // parent classes are handled next.
            let all_base_classes: Vec<(String, u64)> = self
                .collect_all_base_classes(bv)
                .into_iter()
                .filter(|(name, _)| !self.base_classes.contains(name))
                .collect::<_>();

            log::info!("{:#x?}", all_base_classes);

            // Iterate over top-level parents, finding any inherited classes
            let mut vtable_names_and_offsets = vec![];
            let mut current_offset = 0u64;
            for top_level_base in self.base_classes.iter() {
                let width = get_type_width_by_name(top_level_base, bv)
                    .expect(&format!("Could not find width for {}", top_level_base));

                // Check if there are any inherited classes at top-level parent class's offset.
                // If not, this parent does not inherit from other classes. Make the inherited
                // vtable name simply <parent_name>_vtable. Otherwise, the inherited name is the
                // <child_name>_vtable_<parent_name>
                match all_base_classes
                    .iter()
                    .find(|(_, off)| *off == current_offset)
                {
                    None => vtable_names_and_offsets.push((
                        format!("{}_vtable_{top_level_base}", self.get_full_name()),
                        format!("{top_level_base}_vtable"),
                        current_offset,
                    )),
                    Some(parent_name) => vtable_names_and_offsets.push((
                        format!("{}_vtable_{}", self.get_full_name(), top_level_base),
                        format!("{top_level_base}_vtable_{}", parent_name.0),
                        current_offset,
                    )),
                }

                for (base, off) in all_base_classes
                    .iter()
                    .filter(|(_, off)| *off > current_offset && *off < current_offset + width)
                {
                    vtable_names_and_offsets.push((
                        format!("{}_vtable_{base}", self.get_full_name()),
                        format!("{top_level_base}_vtable_{base}"),
                        *off,
                    ));
                }
                current_offset += width;
            }

            log::info!(
                "Got inherited class vtable_names_and_offsets {:#x?}",
                vtable_names_and_offsets
            );

            for (i, (new_vtable_name, inherited_vtable_name, offset)) in
                vtable_names_and_offsets.iter().enumerate()
            {
                let mut vtable_builder = StructureBuilder::new();

                // Add base class vtable as base structure and set proper width
                let mut base_vtable_width = 0u64;
                if let Some(base_vtable_type_id) = bv.type_id_by_name(inherited_vtable_name) {
                    let base_vtable_ref = NamedTypeReference::new_with_id(
                        NamedTypeReferenceClass::StructNamedTypeClass,
                        &base_vtable_type_id,
                        inherited_vtable_name,
                    );

                    // Get the width of the base vtable.
                    if let Some(width) = get_type_width_by_name(&inherited_vtable_name, bv) {
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
                self.process_vtable_methods_for_base(
                    &mut vtable_builder,
                    &inherited_vtable_name,
                    i == 0, // Only process methods for the first base class
                    bv,
                )?;

                // Define the inherited vtable structure
                let vtable_structure = Type::structure(&vtable_builder.finalize());
                bv.define_user_type(new_vtable_name, &vtable_structure);
                vtable_names.push((new_vtable_name.clone(), *offset));
            }
            if !self.vtable_methods.is_empty() {
                self.vtable_methods.iter().for_each(|x| {
                    log::warn!(
                        "Failed to define vtable method: {}::{}, override {:?}",
                        self.name,
                        x.0.name(),
                        x.1
                    );
                })
            }
        }

        log::info!("Got vtable names and offsets: {:#x?}", vtable_names);

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

        // Step 2: Insert vtables.
        // This includes overriding inherited class vtables (including indirect inheritance)
        for (vtable_name, offset) in vtable_names {
            // Create vtable pointer
            let vtable_ptr = Type::pointer(
                &bv.default_arch().expect("Could not find default arch"),
                &Type::named_type(&NamedTypeReference::new(
                    NamedTypeReferenceClass::StructNamedTypeClass,
                    &vtable_name,
                )),
            );

            let vtable_member_name = match vtable_name.find("_vtable") {
                Some(pos) => &vtable_name[pos + 1..],
                None => &vtable_name[..],
            };
            // Insert vtable at the correct offset for this base class
            class_builder.insert(
                vtable_ptr.as_ref(),
                &vtable_member_name,
                offset,
                true, // overwrite existing
                MemberAccess::PublicAccess,
                MemberScope::NoScope,
            );
            // }
        }

        // Step 3: Handle member variable overrides and add member variables specific to this class
        let all_base_classes = self.collect_all_base_classes(bv);
        for (member, override_info) in &self.member_variables {
            if let Some(override_str) = override_info {
                // This member overrides a base class member
                // Look for the base class in all collected base classes (including indirect ones)
                if let Some((_, base_offset)) =
                    Self::parse_member_override_offset(override_str, &all_base_classes, bv)
                {
                    // Find the offset of this base class in the collected base classes
                    // let mut actual_offset = base_offset;
                    // for (collected_base, collected_offset) in &all_base_classes {
                    //     if collected_base == &base_class {
                    //         actual_offset += collected_offset;
                    //         break;
                    //     }
                    // }
                    log::debug!("Found override for {override_str} at offset {base_offset:x}");

                    // Insert the overriding member at the calculated offset
                    member.define(Some(base_offset), &mut class_builder, bv)?;
                    continue;
                }
            }

            // No override, append normally
            member.define(None, &mut class_builder, bv)?;
        }

        // Define the class structure
        let class_structure = Type::structure(&class_builder.finalize());
        let full_name = self.get_full_name();
        bv.define_user_type(&full_name, &class_structure);

        Ok(())
    }

    /// Gets the full name including namespace prefix
    pub fn get_full_name(&self) -> String {
        if self.namespace_path.is_empty() {
            self.name.clone()
        } else {
            format!("{}::{}", self.namespace_path.join("::"), self.name)
        }
    }
}
