use binaryninja::binary_view::BinaryView;
use binaryninja::binary_view::BinaryViewExt;
use binaryninja::rc::Ref;
use binaryninja::types::{NamedTypeReference, NamedTypeReferenceClass, Type};

/// Maps C++ primitive type names to Binary Ninja types
///
/// # Arguments
/// * `s` - The C++ primitive type name as a string
///
/// # Returns
/// * `Some(Ref<Type>)` - Binary Ninja type reference if the type is primitive
/// * `None` - If the type is not a recognized primitive
pub fn is_primitive(s: &str) -> Option<Ref<Type>> {
    match s {
        "char" => Some(Type::int(1, true)),
        "unsigned char" => Some(Type::int(1, false)),
        "int8_t" => Some(Type::int(1, true)),
        "uint8_t" => Some(Type::int(1, false)),
        "int16_t" => Some(Type::int(2, true)),
        "uint16_t" => Some(Type::int(2, false)),
        "int32_t" => Some(Type::int(4, true)),
        "uint32_t" => Some(Type::int(4, false)),
        "int" => Some(Type::int(4, true)),
        "unsigned int" => Some(Type::int(4, false)),
        "int64_t" => Some(Type::int(8, true)),
        "uint64_t" => Some(Type::int(8, false)),
        "float" => Some(Type::int(4, true)),
        "double" => Some(Type::int(8, true)),
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

pub fn get_type_by_name(name: &str, bv: &BinaryView) -> Option<Ref<Type>> {
    let id = bv.type_id_by_name(name)?;
    bv.type_by_id(&id)
}

pub fn get_non_primitive_type_by_name(name: &str, bv: &BinaryView) -> Option<Ref<Type>> {
    let type_id = bv.type_id_by_name(name)?;
    let named_ref = NamedTypeReference::new_with_id(
        NamedTypeReferenceClass::StructNamedTypeClass,
        &type_id,
        name,
    );
    // Hack: `Type::named_type(&named_ref)` always returns
    // a reference with width 0, making any member structs
    // (that are not pointers to structs) appear with 0 size.
    // Workaround using `Type::named_type_from_type` after
    // fetching the type with `type_by_ref` using the reference
    // above
    Some(Type::named_type_from_type(
        named_ref.name(),
        bv.type_by_ref(&named_ref)?.as_ref(),
    ))
}

/// Parses a template member definition into individual tokens
///
/// # Arguments
/// * `s` - The template member definition string, like
/// `struct_name<type1, type2>** member_name`
///
/// # Returns
/// Vector of tokens from the definition
pub fn parse_template_member_definition(s: &str) -> Vec<String> {
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
pub fn parse_template_instantiation(s: &str) -> Result<Option<Vec<String>>, String> {
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

    if !stack.is_empty() {
        return Err("Could not find balanced < and > in template instantiation".to_string());
    }

    // get last item if not empty
    let curr = curr.trim().to_string();
    if !curr.is_empty() {
        typenames.push(curr);
    }

    Ok(Some(typenames))
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
pub fn parse_template_definition(s: &str) -> Option<Vec<String>> {
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

/// Parses a templated typedef assignment like "AA = Abc<T, uint32_t>"
///
/// Extracts the typedef name, target template name, and template parameters
/// from assignment statements used in templated typedef declarations.
///
/// # Arguments
/// * `s` - The assignment string, e.g., "AA = Abc<T, uint32_t>"
///
/// # Returns
/// `Ok(Some((typedef_name, target_template_name, template_params)))` if parsing succeeds, `None` if it fails
pub fn parse_typedef_assignment(s: &str) -> Result<Option<(String, String, Vec<String>)>, String> {
    // Find the '=' separator
    let parts: Vec<&str> = s.splitn(2, '=').collect();
    if parts.len() != 2 {
        return Err("Could not find = in typedef assignment".to_string());
    }

    let typedef_name = parts[0].trim().to_string();
    let target_type = parts[1].trim();

    // Parse the target type to extract template name and parameters
    if let Some(start) = target_type.find('<') {
        let template_name = target_type[..start].trim().to_string();
        let end = target_type
            .rfind('>')
            .ok_or("Could not find closing > in typedef assignment".to_string())?;
        let params_str = &target_type[start + 1..end];

        // Use existing function to parse template parameters
        let template_params = parse_template_instantiation(params_str)?
            .ok_or("Could not parse template instantiation in typedef assignment".to_string())?;

        Ok(Some((typedef_name, template_name, template_params)))
    } else {
        // Non-templated target (shouldn't happen for templated typedefs, but handle gracefully)
        Err("Could not find starting < in typedef assignment".to_string())
    }
}

pub fn strip_type_prefix(s: &str) -> Option<&str> {
    // Strip type keywords
    if s.starts_with("struct ") {
        s.strip_prefix("struct ")
    } else if s.starts_with("class ") {
        s.strip_prefix("class ")
    } else if s.starts_with("enum ") {
        s.strip_prefix("enum ")
    } else if s.starts_with("template ") {
        s.strip_prefix("template ")
    } else {
        Some(s)
    }
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
pub fn parse_name(def: &str) -> Option<String> {
    let mut s = def.trim();

    s = strip_type_prefix(s)?;

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

pub fn parse_decimal_or_hex(num: &str) -> Result<Option<i64>, String> {
    // Parse decimal or hexadecimal
    if num.starts_with("0x") || num.starts_with("0X") {
        i64::from_str_radix(&num[2..], 16)
            .map_err(|e| format!("Could not parse {num} into an i64: {e}"))
            .map(|x| Some(x))
    } else if num.starts_with("-0x") || num.starts_with("-0X") {
        let res = i64::from_str_radix(&num[3..], 16)
            .map_err(|e| format!("Could not parse {num} into an i64: {e:?}"))?;
        Ok(Some(-res))
    } else {
        Ok(Some(num.parse::<i64>().map_err(|e| {
            format!("Could not parse {num} into an i64: {e:?}")
        })?))
    }
}

/// Parses the __ptr_offset(X) directive from a structure definition
///
/// # Arguments
/// * `def` - The structure definition string
///
/// # Returns
/// The parsed offset value as u16, or None if not found
pub fn parse_ptr_offset(def: &str) -> Result<Option<i64>, String> {
    use regex::Regex;

    let re = Regex::new(r"__ptr_offset\((-?0x[0-9a-fA-F]+|-?\d+)\)").unwrap();
    if let Some(captures) = re.captures(def) {
        let offset_str = captures
            .get(1)
            .ok_or("Could not get first capture group while parsing pointer offset")?
            .as_str();
        return parse_decimal_or_hex(offset_str);
    }
    Ok(None)
}

/// Determines the pointer depth and extracts the suffix from a type definition
///
/// # Arguments
/// * `def` - The type definition string
///
/// # Returns
/// A tuple of (pointer_depth, remaining_suffix)
pub fn get_pointer_depth(def: &str) -> (u8, String) {
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

/// Parses array size from a member definition
///
/// Handles array size syntax like `[0x10]`, `[16]`, or `[256]`
///
/// # Arguments
/// * `def` - The member definition string containing array syntax
///
/// # Returns
/// `Some((element_name, array_size))` if array syntax is found, `None` otherwise
pub fn parse_array_size(def: &str) -> Option<(String, u64)> {
    if let Some(start) = def.find('[') {
        if let Some(end) = def.find(']') {
            let size_str = &def[start + 1..end];
            let array_size = if size_str.starts_with("0x") || size_str.starts_with("0X") {
                // Parse hexadecimal
                u64::from_str_radix(&size_str[2..], 16).ok()?
            } else {
                // Parse decimal
                size_str.parse::<u64>().ok()?
            };
            let element_name = def[..start].trim().to_string();
            return Some((element_name, array_size));
        }
    }
    None
}

/// Parses a member definition into type, name, pointer depth, and array info
///
/// # Arguments
/// * `def` - The member definition string
///
/// # Returns
/// `Some((type, name, pointer_depth, array_size))` if parsing succeeds, `None` otherwise
/// `array_size` is `Some(size)` for arrays, `None` for non-arrays
pub fn parse_member_definition(def: &str) -> Option<(String, String, u8, Option<u64>)> {
    let mut def = def.trim();
    if let Some(trimmed) = def.strip_suffix(";") {
        def = trimmed.trim();
    }
    def = strip_type_prefix(def)?;

    // Check for array syntax first
    if let Some((element_part, array_size)) = parse_array_size(def) {
        let name = parse_member_name(&element_part).unwrap_or("".to_string());
        let mut type_part = element_part.strip_suffix(&name).unwrap();
        let (depth, suffix) = get_pointer_depth(type_part);
        if !suffix.is_empty() {
            type_part = type_part.strip_suffix(&suffix).unwrap();
        }
        return Some((type_part.to_string(), name, depth, Some(array_size)));
    }

    // Handle non-array types
    let name = parse_member_name(def).unwrap_or("".to_string());
    def = def.strip_suffix(&name).unwrap();
    let (depth, suffix) = get_pointer_depth(def);
    if !suffix.is_empty() {
        def = def.strip_suffix(&suffix).unwrap();
    }

    Some((def.to_string(), name, depth, None))
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
pub fn parse_member_name(s: &str) -> Option<String> {
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
pub fn find_next_token(s: &str) -> Option<(usize, char, &str)> {
    // Check for line comments
    if s.trim().starts_with("//") {
        for (i, c) in s.char_indices() {
            if c == '\n' {
                return Some((i, '/', &s[..i]));
            }
        }
    }
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
pub fn find_closing_token(s: &str, token: char) -> Option<(usize, char, &str)> {
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
