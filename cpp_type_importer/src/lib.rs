use binaryninja::architecture::CoreArchitecture;
use binaryninja::binary_view::BinaryViewExt;
use binaryninja::command::{register_command, Command};
use binaryninja::high_level_il::operation::DerefFieldSsa;
// use binaryninja::logger::Logger;
use binaryninja::rc::Ref;
use binaryninja::types::{
    BaseStructure, Enumeration, EnumerationBuilder, FunctionParameter, MemberAccess, MemberScope,
    NamedTypeReference, NamedTypeReferenceClass, StructureBuilder, Type,
};
use binaryninja::update::time_since_last_update_check;
use binaryninja::{architecture::Architecture, binary_view::BinaryView};
use log::{error, info, LevelFilter};
use regex::Regex;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

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

#[derive(Debug, Clone)]
pub struct Template {
    name: String,
    typenames: Vec<String>,
    body: String,
}

#[derive(Debug, Clone)]
pub struct Enum {
    name: String,
    size: u64,
    values: Vec<(String, u64)>,
}

impl<'a> Enum {
    pub fn new(name: &str, size: u64, body: &str) -> Self {
        let mut values = Vec::new();
        let mut current_value = 0u64;

        for line in body.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let line = line.trim_end_matches(',');

            if let Some(eq_pos) = line.find('=') {
                // Parse explicit assignment like "GGGZERO=1"
                let name = line[..eq_pos].trim().to_string();
                let value_str = line[eq_pos + 1..].trim();

                // Parse the numeric value
                if let Ok(assigned_value) = value_str.parse::<u64>() {
                    current_value = assigned_value;
                } else {
                    // If we can't parse it, default to current_value
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

    pub fn define(&self, bv: &BinaryView) -> bool {
        // Create an enumeration builder
        let mut builder = EnumerationBuilder::new();

        // Add each enum value to the builder with its correct numeric value
        for (name, value) in &self.values {
            builder.insert(name, *value);
        }

        // Finalize the enumeration
        let enumeration = builder.finalize();

        // Create the enum type with the specified width
        let width = std::num::NonZeroUsize::new(self.size as usize)
            .unwrap_or_else(|| std::num::NonZeroUsize::new(4).unwrap());
        let enum_type = Type::enumeration(&enumeration, width, false);

        // Define the type in Binary Ninja
        bv.define_user_type(&self.name, &enum_type);
        true
    }
}

impl<'a> Template {
    pub fn new(def: &str, body: &str, typenames: Vec<String>) -> Self {
        let name = parse_name(def).expect(&format!("Could not parse name from {def}"));
        Self {
            name,
            typenames,
            body: body.to_string(),
        }
    }
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
            println!("Member {member}");
            members.push(Member::new(
                member,
                bv,
                Some(&self.typenames),
                Some(&typenames),
            ));
        }
        let mut name = self.name.clone();
        name.push('<');
        name.push_str(&typenames.join(", "));
        name.push('>');
        Structure::new_from_members(name, members, 0).define(bv);
    }
}

#[derive(Debug)]
pub struct Structure {
    name: String,
    members: Vec<Member>,
    offset: u16,
}

impl<'a> Structure {
    pub fn new<'b>(def: &str, body: &str, bv: &'a BinaryView) -> Self {
        let name = parse_name(def).expect(&format!("Could not parse definition {def} for name"));
        let mut members = vec![];
        for member in body.lines() {
            members.push(Member::new(member, bv, None, None));
        }

        Self {
            name,
            members,
            offset: 0,
        }
    }
    pub fn new_from_members(name: String, members: Vec<Member>, offset: u16) -> Self {
        Self {
            name,
            members,
            offset,
        }
    }
    pub fn define<'b>(&mut self, bv: &'a BinaryView) -> bool {
        let mut builder = StructureBuilder::new();
        for m in self.members.iter_mut() {
            match m {
                Member::Basic {
                    name,
                    typ,
                    comments,
                } => {
                    builder.append(
                        typ.as_ref(),
                        &name.clone(),
                        MemberAccess::PublicAccess,
                        MemberScope::NoScope,
                    );
                }
                Member::Function { name, ret, args } => {
                    println!("NAME: {name}");
                    let mut v = vec![];
                    for (arg_name, arg_type) in args {
                        v.push(FunctionParameter::new(
                            arg_type.clone(),
                            arg_name.clone(),
                            None,
                        ));
                    }
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
                _ => (),
            }
        }
        let s = Type::structure(&builder.finalize());
        dbg!(format!("Defining {}", self.name));
        bv.define_user_type(&self.name, &s);
        true
    }
}

#[derive(Debug)]
pub struct Class {
    name: String,
    vtable_methods: Vec<(Member, Option<String>)>, // Store method and override info
    member_variables: Vec<(Member, Option<String>)>, // Store member and override info
    base_classes: Vec<String>,                     // Store base class names for now
}

impl<'a> Class {
    pub fn new(def: &str, body: &str, bv: &'a BinaryView) -> Self {
        // Parse class name and inheritance with regex
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
            // Fallback to existing parse_name logic
            let name = parse_name(def).expect(&format!("Could not parse class name from {def}"));
            (name, Vec::new())
        };

        // Forward-declare the class as a structure so it can be referenced in constructor signatures
        let forward_decl = Type::structure(&StructureBuilder::new().finalize());
        bv.define_user_type(&name, &forward_decl);

        let mut vtable_methods = Vec::new();
        let mut member_variables = Vec::new();
        let mut in_vtable = true;

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
                let override_regex = Regex::new(r"//\s*;\s*override\s+(.+?);").unwrap();
                let override_info = if let Some(captures) = override_regex.captures(line) {
                    Some(captures.get(1).unwrap().as_str().to_string())
                } else {
                    None
                };

                // Remove override comment from the line
                let clean_line = override_regex.replace(line, "").trim().to_string();
                let member = Member::new(&clean_line, bv, None, None);
                member_variables.push((member, override_info));
            }
        }

        Self {
            name,
            vtable_methods,
            member_variables,
            base_classes,
        }
    }

    fn parse_vtable_method(
        line: &str,
        bv: &'a BinaryView,
        class_name: &str,
    ) -> Option<(Member, Option<String>)> {
        // Parse offset comment with regex
        let offset_regex = Regex::new(r"//\s*;\s*offset=(-?\d+)").unwrap();
        let this_offset = if let Some(captures) = offset_regex.captures(line) {
            let offset_str = captures.get(1).unwrap().as_str();
            if offset_str == "-1" {
                None // Static method
            } else {
                offset_str.parse::<usize>().ok()
            }
        } else {
            Some(0) // Default: this is at position 0
        };

        // Parse override comment with regex
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

        // For regular methods, inject this pointer at the specified offset
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

    fn process_vtable_methods_for_base(
        vtable_methods: &[(Member, Option<String>)],
        vtable_builder: &mut StructureBuilder,
        base_class: &str,
        process_non_overriding: bool,
        bv: &'a BinaryView,
    ) {
        for (method, override_info) in vtable_methods {
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
                    println!("OVERRIDE STR {override_str}");
                    if Self::is_override_for_base_class(override_str, base_class) {
                        println!("FINDING OFFSET IN THIS CURRENT CLASS {base_class}");
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

    fn is_override_for_base_class(override_str: &str, base_class: &str) -> bool {
        // Check if the override string contains the base class vtable name
        let base_vtable_name = format!("{}_vtable", base_class);
        override_str.contains(&base_vtable_name)
    }

    fn parse_override_offset(
        override_str: &str,
        base_class: &str,
        bv: &'a BinaryView,
    ) -> Option<u64> {
        // Parse override string like "void (* HHH_vtable::HHH)(struct HHH* this)"
        // Look for the base class vtable and method name
        let base_vtable_name = format!("{}_vtable", base_class);

        // Find the vtable type and look for the method
        if let Some(base_vtable_type_id) = bv.type_id_by_name(&base_vtable_name) {
            if let Some(base_vtable_type) = bv.type_by_id(&base_vtable_type_id) {
                // Get the structure members to find the method offset
                if let Some(structure) = base_vtable_type.get_structure() {
                    // Parse the method name from the override string
                    if let Some(cleaned_override) =
                        Self::extract_method_name_from_override(override_str)
                    {
                        // Find the method in the base vtable structure by reconstructing the signature
                        for (i, member) in structure.members().iter().enumerate() {
                            let mut contents = member.ty.contents.to_string();

                            // Find (*) and insert the member name after the *
                            if let Some(star_pos) = contents.find("(*)") {
                                contents.insert_str(star_pos + 2, &format!(" {}", member.name));
                            }

                            println!(
                                "Checking cleaned override '{}' against reconstructed '{}'",
                                cleaned_override, contents
                            );

                            if cleaned_override == contents {
                                println!("GOT MATCH");
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

    fn extract_method_name_from_override(override_str: &str) -> Option<String> {
        // Parse override string like "void (* HHH_vtable::HHH)(struct HHH* this)"
        // Remove the inherited XXX_vtable:: prefix while keeping the rest
        let regex = Regex::new(r"\w+_vtable::").unwrap();
        let cleaned = regex.replace_all(override_str, "");
        Some(cleaned.to_string())
    }

    fn extract_member_from_override(override_str: &str, base_class: &str) -> Option<String> {
        // For member overrides, remove the base_class:: prefix from anywhere in the string
        let prefix = format!("{}::", base_class);
        let cleaned = override_str.replace(&prefix, "");
        Some(cleaned)
    }

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
                if let Some(base_type_id) = bv.type_id_by_name(base_class) {
                    if let Some(base_type) = bv.type_by_id(&base_type_id) {
                        if let Some(structure) = base_type.get_structure() {
                            // Find the member in the base class structure
                            for member in structure.members() {
                                let mut member_type_contents = member.ty.contents.to_string();
                                let check_against =
                                    if let Some(star_pos) = member_type_contents.find("(*)") {
                                        member_type_contents
                                            .insert_str(star_pos + 2, &format!(" {}", member.name));
                                        member_type_contents
                                    } else {
                                        format!("{member_type_contents} {}", member.name)
                                    };

                                println!(
                                    "Checking member override '{}' against base class '{}' member '{}' with type '{}'",
                                    cleaned_override, base_class, member.name, check_against
                                );

                                if cleaned_override == check_against {
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

    pub fn define(&self, bv: &'a BinaryView) -> bool {
        // Create vtables for each base class
        let mut vtable_names = Vec::new();

        if self.base_classes.is_empty() {
            // No inheritance - create regular vtable
            let vtable_name = format!("{}_vtable", self.name);
            let mut vtable_builder = StructureBuilder::new();

            for (method, _override_info) in &self.vtable_methods {
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

                    // Get the width of the base vtable
                    if let Some(base_vtable_type) = bv.type_by_id(&base_vtable_type_id) {
                        base_vtable_width = base_vtable_type.width();
                    }

                    let base_struct = BaseStructure::new(base_vtable_ref, 0, base_vtable_width);
                    vtable_builder.base_structures(&[base_struct]);

                    // Set the vtable builder width to account for the base vtable
                    vtable_builder.width(base_vtable_width);
                }

                // Process vtable methods for this base class
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
                        true,
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
                    true,
                    MemberAccess::PublicAccess,
                    MemberScope::NoScope,
                );
            }
        }

        // Step 3: Handle member variable overrides and add member variables specific to this class
        let mut cumulative_base_offset = 0u64;
        for (member, override_info) in &self.member_variables {
            if let Some(override_str) = override_info {
                // This member overrides a base class member
                if let Some((base_class, base_offset)) =
                    Self::parse_member_override_offset(override_str, &self.base_classes, bv)
                {
                    // Calculate the actual offset by adding the base class offset
                    let mut actual_offset = base_offset;
                    for (i, base) in self.base_classes.iter().enumerate() {
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
                _ => (),
            }
        }

        // Define the class structure
        let class_structure = Type::structure(&class_builder.finalize());
        bv.define_user_type(&self.name, &class_structure);

        true
    }
}

#[derive(Debug)]
pub enum Member {
    Basic {
        name: String,
        typ: Ref<Type>,
        comments: Vec<String>,
    },
    Function {
        name: String,
        ret: Ref<Type>,
        args: Vec<(String, Ref<Type>)>,
    },
    Template {
        name: String,
        args: Vec<String>,
        comments: Vec<String>,
    },
}

impl Member {
    fn define_type(t: &str, depth: u8, bv: &BinaryView) -> Ref<Type> {
        println!("Defining type {t}");
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
                println!("Found named type {:?}", named_ref);
                // Hack: `Type::named_type(&named_ref)` always returns
                // a reference with width 0, making any member structs
                // (that are not pointers to structs) appear with 0 size
                Type::named_type_from_type(
                    named_ref.name(),
                    bv.type_by_ref(&named_ref)
                        .expect("Could not find type by ref")
                        .as_ref(),
                )
            } else {
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
    fn new(
        def: &str,
        bv: &BinaryView,
        template_members: Option<&Vec<String>>,
        template_defs: Option<&Vec<String>>,
    ) -> Self {
        let (mut typ, name, depth) =
            parse_member_definition(def).expect("Could not parse member definition");
        dbg!(format!("GOT MEMBER DEFINITION {typ} {name} {depth}"));
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
                println!("{tokens:?}");

                // Replace template parameters with their concrete types
                for token in &mut tokens {
                    if let Some(index) = t_members.iter().position(|t| t == token) {
                        *token = t_defs[index].clone();
                    }
                }

                // Reconstruct the type string with substituted parameters
                def = tokens.join("");
                println!("Substituted type: {def}");
            }
            // Try to match function definition: return_type (*name)(args)
            let func_regex = Regex::new(r"(.*) \(\*(.*)\)\((.*)\)").unwrap();
            if let Some(captures) = func_regex.captures(&def) {
                let return_type = captures.get(1).unwrap().as_str().trim();
                let name = captures.get(2).unwrap().as_str().trim();
                let args = captures.get(3).unwrap().as_str().trim();

                dbg!(format!(
                    "GOT FUNCTION DEFINITION: return_type={}, name={}, args={:#}",
                    return_type, name, args
                ));
                let (return_type, _, depth) = parse_member_definition(return_type)
                    .expect("Could not parse function member return type");
                println!("{return_type} {depth}");
                let args = parse_template_instantiation(args)
                    .expect("Could not parse function member args");
                let mut defined_args = vec![];
                println!("args={:?}", args);
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

// Parses template instantiation `temp<type1, type2<type3, type4>>`, etc. The input
// should be the text within the opening `<` and closing `>`.
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

/// Parses for templated typenames declared between `< ... >`. Input `s` should be
/// the contents between the angle brackets, where each templated name is separated by
/// a comma.
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

    // find closing '>', so the remaining string should appear <class|struct|etc> <name>
    // let rest = &rest[rest.find('>')? + 1..].trim();
    // let rest = &rest[rest.find(' ')? + 1..].trim();
    // rest should now be just the name
    Some(typenames)
}

/// Parses a class, struct, or enum definition where the string `def` is formatted
/// like <type> <name>. This should be a forward declaration or a structure definition
/// preceding the opening brace.
fn parse_name(def: &str) -> Option<String> {
    let mut s = def.trim();
    if s.starts_with("struct ") {
        s = s.strip_prefix("struct ")?;
    } else if s.starts_with("class ") {
        s = s.strip_prefix("class ")?;
    } else if s.starts_with("enum ") {
        s = s.strip_prefix("enum ")?;
    }
    Some(s.to_string())
}

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

/// Parses the name from a member definition, such as `struct B* C`. Assumes that
/// the delineating characters between the name and the type are any combination of
/// `*` and ` `. Assumes there is no trailing `;`.
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

fn find_next_token(s: &str) -> Option<(usize, char, &str)> {
    for (i, c) in s.char_indices() {
        if c == '{' || c == ';' || c == '<' || c == '"' {
            return Some((i, c, &s[..i]));
        }
    }
    None
}

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

pub struct Parser<'a> {
    bv: &'a BinaryView,
    arch: CoreArchitecture,
    // templates,
    // classes
    // structs
}

impl<'a> Parser<'a> {
    /// Creates a new Parser instance with the provided BinaryView
    pub fn new(bv: &'a BinaryView) -> Self {
        let arch = bv
            .default_arch()
            .expect("Could not get default architecture");
        Self { bv, arch }
    }

    /// Parses C++ header content and imports types into Binary Ninja
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

struct ImportCppTypesCommand;

impl Command for ImportCppTypesCommand {
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

    fn valid(&self, _view: &BinaryView) -> bool {
        // Command is always valid
        true
    }
}

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
    use binaryninja::binary_view::{BinaryView, BinaryViewBase, BinaryViewExt};
    use binaryninja::headless::Session;
    use binaryninja::rc::Ref;
    use std::fs::File;
    use std::io::{self, Read};
    use std::path::PathBuf;

    // fn get_binary_view() -> Ref<BinaryView> {

    // println!("Filename:  `{}`", bv.file().filename());
    // println!("File size: `{:#x}`", bv.len());
    // println!("Function count: {}", bv.functions().len());
    // bv
    // }

    use crate::{find_next_token, parse_template_definition, Parser};

    #[test]
    fn test_parsing() {
        use binaryninja::binary_view::{BinaryView, BinaryViewBase, BinaryViewExt};
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
        let p = Parser {
            arch: bv
                .as_ref()
                .default_arch()
                .expect("Could not get default architecture")
                .clone(),
            bv: bv.as_ref(),
        };
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
