use binaryninja::architecture::CoreArchitecture;
use binaryninja::binary_view::BinaryViewExt;
use binaryninja::command::{register_command, Command};
use binaryninja::high_level_il::operation::DerefFieldSsa;
// use binaryninja::logger::Logger;
use binaryninja::rc::Ref;
use binaryninja::types::{
    FunctionParameter, MemberAccess, MemberScope, NamedTypeReference, NamedTypeReferenceClass,
    StructureBuilder, Type,
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

impl Template {
    pub fn new(def: &str, body: &str, typenames: Vec<String>) -> Self {
        let name = parse_name(def).expect(&format!("Could not parse name from {def}"));
        Self {
            name,
            typenames,
            body: body.to_string(),
        }
    }
    pub fn define(&self, typenames: Vec<String>) {
        assert_eq!(
            typenames.len(),
            self.typenames.len(),
            "Provided typenames length does not match expected typenames length"
        );
        for member in self.body.lines() {
            if let Some((typ, name, depth)) = parse_member_definition(member) {
                dbg!(format!("GOT MEMBER DEFINITION {typ} {name} {depth}"));
            } else {
                dbg!(format!("Failed to parse member defintion {member}"));
            }
        }
        let mut members = Vec::<Member>::new();
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
            members.push(Member::new(member, bv));
        }

        Self {
            name,
            members,
            offset: 0,
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
                        name.clone(),
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
                        name.clone(),
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

pub struct Class {
    name: String,
    members: Vec<Member>,
    fns: Vec<Member>,
    base_classes: Vec<Class>,
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
                    type_id,
                    t,
                );
                println!("Found named type {:?}", named_ref);
                Type::named_type(&named_ref)
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
    fn new(def: &str, bv: &BinaryView) -> Self {
        let (typ, name, depth) =
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
            // Try to match function definition: return_type (*name)(args)
            let func_regex = Regex::new(r"(.*) \(\*(.*)\)\((.*)\)").unwrap();
            if let Some(captures) = func_regex.captures(def) {
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
                    parse_member_definition(def).expect(&format!("Could not parse {def}"));
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
        s.strip_prefix("enum ")?;
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

// impl Member {
//     fn find_name(def: &str) -> (String, &str) {
//         for (i, c) in def.chars().rev().enumerate() {
//             match c {
//                 '*' | ' ' => {
//                     return (def[def.len() - i..].to_string(), &def[..def.len() - i]);
//                 }
//                 _ => continue,
//             }
//         }
//         (String::new(), &def[..])
//     }
//     fn find_pointer_depth(def: &str) -> (u8, &str) {
//         let mut depth = 0u8;
//         for (i, c) in def.chars().rev().enumerate() {
//             match c {
//                 '*' => depth += 1,
//                 ' ' => continue,
//                 _ => return (depth, &def[..def.len() - i]),
//             }
//         }
//         return (depth, &def[..]);
//     }
//     fn parse_basic(def: &str) -> Self {
//         let (name, rest) = Member::find_name(def);
//         // TODO get array from name
//         let (depth, rest) = Member::find_pointer_depth(rest);
//         println!("member name: {name}\nType: {}\nDepth: {depth}", rest.trim());
//         let mut t: Ref<Type> = is_primitive(rest).unwrap_or(Type::void());
//         // for _ in 0..depth {
//         //     t = Type::pointer(Architecture::get("armv7").expect(), t.as_ref());
//         // }
//         Self::Basic {
//             name: name.to_string(),
//             comments: vec![],
//             typ: t,
//         }
//     }
//     pub fn parse(def: &str, templated_types: Vec<String>) -> Self {
//         loop {
//             let (i, c, s) = find_next_token(def).expect("Could not find token in Member::parse");
//             match c {
//                 // Simple definition
//                 ';' => {
//                     return Member::parse_basic(s.trim());
//                 }
//                 '<' => {}
//                 _ => panic!("Unexpected starting token {c} in Member definition"),
//             }
//         }
//     }
// }

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
                let (i2, _, mut s2) =
                    find_closing_token(&contents[idx..], c).expect("Could not find closing token");
                s2 = s2.trim();
                if s.starts_with("#include") {
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
                        } else {
                            let (i3, _, mut s3) =
                                find_closing_token(&contents[idx + i2 + 1..], ';').expect(
                                    "Could not find closing token for template instantiation",
                                );
                            if let Some(stripped) = s3.strip_suffix('>') {
                                s3 = stripped;
                            }
                            let mut instant = s2.to_string();
                            instant.push('>');
                            instant.push_str(s3);
                            dbg!(format!("Got template instantiation {instant}"));

                            let typenames = parse_template_instantiation(&instant)
                                .expect(&format!("Could not parse template definitions {s2}>{s3}"));
                            dbg!(format!("{typenames:?}"));
                            // check for template named `s` to declare
                            let t = templates
                                .iter()
                                .find(|x| &x.name == s.trim())
                                .expect(&format!("Could not find template {s} for definition"));
                            t.define(typenames);
                            idx += i3 + 1
                        }
                    }
                    ';' => {
                        // forward declaration
                        dbg!(format!("Got forward declaration {s2}"));
                    }
                    '{' => {
                        // class or struct definition
                        if s.starts_with("struct") {
                            dbg!("Got struct");
                            dbg!(format!("{s}: {s2}"));
                            let mut structure = Structure::new(s, s2, self.bv);
                            dbg!(format!("{structure:?}",));
                            structure.define(self.bv);

                            // for l in s2.lines() {
                            //     dbg!(format!("Got Member {:#?}", Member::parse(l, vec![])));
                            // }
                        } else if s.starts_with("class") {
                            dbg!(format!("Got class {s2}"));
                        }
                    }
                    _ => (),
                }
                idx += i2 + 1;
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
