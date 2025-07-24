use binaryninja::binary_view::BinaryView;
use binaryninja::command::{register_command, Command};
use binaryninja::logger::Logger;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

mod utils;
use utils::*;

mod class;
use class::*;

mod member;
use member::*;

mod enumeration;
use enumeration::*;

mod typedef;
use typedef::*;

mod template;
use template::*;

mod structure;
use structure::*;

mod parser;
use parser::*;

// TODO
// 1. Conflicting vtable function names (e.g., MyMethod)
// 2. Add comment lines to middle of structure and class

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
        log::info!("Importing C++ types from test.hpp");

        let path = if let Some(p) = binaryninja::interaction::get_text_line_input(
            "Enter path to directory holding .hpp files",
            "Directory input",
        ) {
            if !Path::new(&p).exists() {
                log::error!("Path {p} does not exist");
                return;
            }
            p
        } else {
            log::error!("Could not get directory input from user");
            return;
        };
        let filenames = if let Some(p) = binaryninja::interaction::get_text_line_input(
            "Enter a comma-separated list of .hpp files",
            "Filename(s) input",
        ) {
            let mut filenames = vec![];
            for f in p.split(",") {
                let mut buf = match PathBuf::from_str(&path) {
                    Ok(buf) => buf,
                    Err(e) => {
                        log::error!("Could not create PathBuf for {path}: {e}");
                        return;
                    }
                };
                buf.push(f);
                if !buf.exists() {
                    log::error!("Path {p} does not exist");
                    return;
                }
                filenames.push(buf);
            }
            filenames
        } else {
            log::error!("Could not get directory input from user");
            return;
        };

        // Read the file contents
        for f in filenames {
            log::info!("Opening file {}", f.display());
            match File::open(&f) {
                Ok(mut file) => {
                    let mut contents = String::new();
                    match file.read_to_string(&mut contents) {
                        Ok(_) => {
                            log::info!("Successfully read {}, parsing C++ types...", f.display());
                            let parser = Parser::new(view);
                            match parser.parse(&contents) {
                                Err(e) => log::error!(
                                    "Could not parse types from file {}: {e}",
                                    f.display()
                                ),
                                Ok(_) => log::info!("C++ type import completed"),
                            }
                        }
                        Err(e) => {
                            log::error!("Failed to read {}: {}", f.display(), e);
                        }
                    }
                }
                Err(e) => {
                    log::error!("Failed to open {}: {}", f.display(), e);
                }
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
        .with_level(log::LevelFilter::Info)
        .init();

    // Register the C++ Type Importer command
    register_command(
        "Import C++ Types",
        "Import C++ types from test.hpp into Binary Ninja's type system",
        ImportCppTypesCommand {},
    );

    log::info!("C++ Type Importer plugin initialized");

    true
}

#[cfg(test)]
mod tests {
    use binaryninja::headless::Session;
    use binaryninja::rc::Ref;
    use binaryninja::types::Type;
    use std::fs::File;
    use std::io::Read;
    use std::path::PathBuf;

    use crate::{
        get_non_primitive_type_by_name, get_type_by_name, get_type_width_by_name, is_primitive,
        parse_name, parse_template_definition, parse_template_instantiation, Class, Enum, Member,
        Parser, Structure, Template, Typedef,
    };
    use binaryninja::binary_view::BinaryViewExt;

    fn get_member_at_struct_offset(
        vtable: &Ref<Type>,
        bv: &binaryninja::binary_view::BinaryView,
        offset: u64,
    ) -> Option<Ref<Type>> {
        let s = vtable.get_structure().unwrap();
        if let Some(f) = s.members().iter().filter(|x| x.offset == offset).next() {
            return Some(f.ty.contents.clone());
        } else {
            for base in s.base_structures() {
                let base_type = bv.type_by_id(&base.ty.id()).unwrap();
                if let Some(member) =
                    get_member_at_struct_offset(&base_type, bv, offset - base.offset)
                {
                    return Some(member);
                }
            }
        }
        None
    }

    fn get_member_name_at_offset(
        vtable: &Ref<Type>,
        bv: &binaryninja::binary_view::BinaryView,
        offset: u64,
    ) -> Option<String> {
        let s = vtable.get_structure().unwrap();
        if let Some(f) = s.members().iter().filter(|x| x.offset == offset).next() {
            return Some(f.name.to_string());
        } else {
            for base in s.base_structures() {
                let base_type = bv.type_by_id(&base.ty.id()).unwrap();
                if let Some(name) = get_member_name_at_offset(&base_type, bv, offset - base.offset)
                {
                    return Some(name);
                }
            }
        }

        None
    }

    fn get_function_argument_type_by_name(f: &Ref<Type>, name: &str) -> Option<Ref<Type>> {
        // f is a PointerTypeClass
        let child = f.child_type()?;
        child
            .contents
            .parameters()?
            .iter()
            .filter(|x| &x.name.to_string() == name)
            .next()
            .map(|x| x.ty.contents.clone())
    }

    #[test]
    fn test_templated_typedef() {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("test.bndb");
        let headless_session = Session::new().expect("Failed to initialize session");
        let bv = headless_session.load(&path).expect("Couldn't open bv");

        // First create the base template that will be aliased
        let base_template = Template::new(
            "Abc",
            "T* a;\nUV** b;",
            vec!["T".to_string(), "UV".to_string()],
            Vec::new(),
        );

        // Test parsing of templated typedef
        let typedef_assignment = "AA = Abc<N, uint32_t>";
        let result = crate::parse_typedef_assignment(typedef_assignment);
        assert!(result.is_ok() && result.as_ref().unwrap().is_some());

        let (typedef_name, template_name, params) = result.unwrap().unwrap();
        assert_eq!(typedef_name, "AA");
        assert_eq!(template_name, "Abc");
        assert_eq!(params, vec!["N".to_string(), "uint32_t".to_string()]);

        // Instantiate typedef
        assert!(base_template
            .define(vec!["char".to_string(), "uint32_t".to_string()], &bv)
            .is_ok());

        // Create templated typedef
        let templated_typedef = Template::new_typedef(
            "AA".to_string(),
            vec!["N".to_string()],
            "Abc".to_string(),
            vec![(0, "N".to_string())],        // N is at position 0
            vec![(1, "uint32_t".to_string())], // uint32_t is at position 1
            Vec::new(),
        );

        // Test instantiation
        let templates = vec![base_template, templated_typedef];
        let typedef_template = &templates[1];

        // This should create AA<char> which internally creates Abc<char, uint32_t>
        if let Err(e) = typedef_template.define(vec!["char".to_string()], bv.as_ref()) {
            println!("Could not define AA<char>: {e}");
            assert!(false);
        }

        // Check that both types were created
        assert!(get_non_primitive_type_by_name("AA<char>", &bv).is_some());
    }

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
        let p = Parser {
            bv: bv.as_ref(),
            namespace_stack: vec![],
        };
        if let Err(e) = p.parse(&contents) {
            println!("Could not parse input file {e}");
            assert!(false);
        }
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
        assert!(typenames.is_ok());
        let typenames = typenames.unwrap();
        assert_eq!(
            typenames.as_ref().unwrap().get(0),
            Some(&"uint32_t".to_string())
        );
        let s = "uint32_t, void*";
        let typenames = parse_template_instantiation(s);
        assert!(typenames.is_ok());
        let typenames = typenames.unwrap();
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
        assert!(typenames.is_ok());
        let typenames = typenames.unwrap();
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
        assert!(typenames.is_ok());
        let typenames = typenames.unwrap();
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
        assert!(typenames.is_ok());
        let typenames = typenames.unwrap();
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
        assert!(is_primitive("float").is_some());
        assert!(is_primitive("double").is_some());

        // Test non-primitive types that should return None
        assert!(is_primitive("MyStruct").is_none());
        assert!(is_primitive("std::string").is_none());
        assert!(is_primitive("vector<int>").is_none());
        assert!(is_primitive("custom_type").is_none());
        assert!(is_primitive("").is_none());
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
            Structure::new("struct2_name", "int32_t x;", bv.as_ref(), Vec::new()).unwrap();
        assert!(basic_struct.define(bv.as_ref()).is_ok());
        assert_eq!(get_type_width_by_name(&"struct2_name", &bv), Some(4));

        // Define class_name as a basic struct
        let mut class_struct = Structure::new(
            "class_name",
            "int32_t field1;\nint32_t field2;",
            bv.as_ref(),
            Vec::new(),
        )
        .unwrap();
        assert!(class_struct.define(bv.as_ref()).is_ok());
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
        if let Err(e) = simple_template.define(vec!["uint32_t".to_string()], bv.as_ref()) {
            println!("Could not define simple_template<uint32_t>: {e}");
            assert!(false);
        }
        assert_eq!(
            get_type_width_by_name(&"structure_name<uint32_t>", &bv),
            Some(4)
        );
        if let Err(e) = simple_template.define(vec!["void*".to_string()], bv.as_ref()) {
            println!("Could not define simple_template<void*>: {e}");
            assert!(false);
        }
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
        if let Err(e) = two_param_template.define(
            vec!["uint32_t".to_string(), "void*".to_string()],
            bv.as_ref(),
        ) {
            println!("Could not define two_param_template<uint32_t, void*>: {e}");
            assert!(false);
        }
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
        if let Err(e) = nested_template.define(
            vec!["void*".to_string(), "struct2_name".to_string()],
            bv.as_ref(),
        ) {
            println!("Could not define nested_template<void*, struct2_name>: {e}");
            assert!(false);
        }
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
        if let Err(e) =
            templated_function.define(vec!["void".to_string(), "int32_t".to_string()], bv.as_ref())
        {
            println!("Could not define templated_function<void, int32_t>: {e}");
            assert!(false);
        }
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
        )
        .unwrap();

        let concrete_member = Member::new(
            "structure_name<uint32_t> member_name",
            bv.as_ref(),
            None,
            None,
            &Vec::new(),
        )
        .unwrap();

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
        )
        .unwrap();

        let concrete_member = Member::new(
            "structure_name_two<uint32_t, void*> member_name",
            bv.as_ref(),
            None,
            None,
            &Vec::new(),
        )
        .unwrap();

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
        )
        .unwrap();

        let concrete_member = Member::new(
            "structure_name<uint32_t>* member_name",
            bv.as_ref(),
            None,
            None,
            &Vec::new(),
        )
        .unwrap();

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
        )
        .unwrap();

        let concrete_function = Member::new(
            "uint32_t (*func_name)(uint32_t param)",
            bv.as_ref(),
            None,
            None,
            &Vec::new(),
        )
        .unwrap();

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
        )
        .unwrap();

        let concrete_function = Member::new(
            "void (*complex_func)(int32_t param1, void* param2)",
            bv.as_ref(),
            None,
            None,
            &Vec::new(),
        )
        .unwrap();

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
        )
        .unwrap();
        assert!(regular_struct.define(bv.as_ref()).is_ok());

        // Create a packed structure without padding
        let packed_struct_def = "struct __attribute__((packed)) PackedStruct";
        let packed_struct_body = "char a;\nint32_t b;\nchar c;";
        let mut packed_struct = Structure::new(
            packed_struct_def,
            packed_struct_body,
            bv.as_ref(),
            Vec::new(),
        )
        .unwrap();
        assert!(packed_struct.define(bv.as_ref()).is_ok());

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
    fn test_structure_offset() {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("test.bndb");
        let headless_session = Session::new().expect("Failed to initialize session");
        let bv = headless_session.load(&path).expect("Couldn't open bv");

        let name = "struct __attribute__((packed)) MyStruct";
        assert_eq!(parse_name(name), Some("MyStruct".to_string()));

        let mut s = Structure::new(
            name,
            r#"// ; __ptr_offset(0x10)
void* a[0x10];
int64_t b;
char c"#,
            bv.as_ref(),
            Vec::new(),
        )
        .unwrap();

        let mut s2 = Structure::new(
            name,
            r#"// ; __ptr_offset(24)
void* a[4];
uint64_t b;
char* c"#,
            bv.as_ref(),
            Vec::new(),
        )
        .unwrap();

        let mut s3 = Structure::new(
            name,
            r#"// ; __ptr_offset(-0x10)
void* a[4];
uint64_t b;
char* c"#,
            bv.as_ref(),
            Vec::new(),
        )
        .unwrap();

        let mut s4 = Structure::new(
            name,
            r#"// ; __ptr_offset(-24)
void* a[4];
uint64_t b;
char* c"#,
            bv.as_ref(),
            Vec::new(),
        )
        .unwrap();

        assert!(s.define(&bv).is_ok());
        assert!(s2.define(&bv).is_ok());
        assert!(s3.define(&bv).is_ok());
        assert!(s4.define(&bv).is_ok());

        // The Rust API does not allow validating the structure's pointer offset
        // after declaration. Instead, just validate prior to definition
        assert_eq!(s.offset, 0x10);
        assert_eq!(s2.offset, 0x18);
        assert_eq!(s3.offset, -0x10);
        assert_eq!(s4.offset, -0x18);
    }

    #[test]
    fn test_array_members() {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("test.bndb");
        let headless_session = Session::new().expect("Failed to initialize session");
        let bv = headless_session.load(&path).expect("Couldn't open bv");

        // Define base structure with array
        let mut base_struct = Structure::new(
            "struct ArrayStruct",
            r#"uint64_t a;
char b[0x10];
uint32_t c[0x8];"#,
            bv.as_ref(),
            Vec::new(),
        )
        .unwrap();
        assert!(base_struct.define(bv.as_ref()).is_ok());

        assert_eq!(get_type_width_by_name("ArrayStruct", &bv), Some(0x38));
        let base_type = get_type_by_name("ArrayStruct", &bv).unwrap();
        let arr_type_b = Type::array(is_primitive("char").unwrap().as_ref(), 0x10);
        let arr_type_c = Type::array(is_primitive("uint32_t").unwrap().as_ref(), 0x8);
        assert_eq!(
            get_member_name_at_offset(&base_type, &bv, 0x0).unwrap(),
            "a".to_string()
        );
        assert_eq!(
            is_primitive("uint64_t").unwrap(),
            get_member_at_struct_offset(&base_type, &bv, 0x0).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&base_type, &bv, 0x8).unwrap(),
            "b".to_string()
        );
        assert_eq!(
            arr_type_b,
            get_member_at_struct_offset(&base_type, &bv, 0x8).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&base_type, &bv, 0x18).unwrap(),
            "c".to_string()
        );
        assert_eq!(
            arr_type_c,
            get_member_at_struct_offset(&base_type, &bv, 0x18).unwrap(),
        );

        // Define base structure with array
        let mut nested_struct = Structure::new(
            "struct ArrayStruct2",
            r#"ArrayStruct a[3];
char* b[0x4];
int32_t c[0x8];"#,
            bv.as_ref(),
            Vec::new(),
        )
        .unwrap();
        assert!(nested_struct.define(bv.as_ref()).is_ok());

        assert_eq!(
            get_type_width_by_name("ArrayStruct2", &bv),
            Some(0xa8 + 0x20 + 0x20)
        );
        let base_type_2 = get_type_by_name("ArrayStruct2", &bv).unwrap();
        let base_type = get_non_primitive_type_by_name("ArrayStruct", &bv).unwrap();
        let arr_type_a = Type::array(base_type.as_ref(), 0x3);
        let arr_type_b = Type::array(
            Type::pointer(
                &bv.default_arch().unwrap(),
                is_primitive("char").unwrap().as_ref(),
            )
            .as_ref(),
            0x4,
        );
        let arr_type_c = Type::array(is_primitive("int32_t").unwrap().as_ref(), 0x8);
        assert_eq!(
            get_member_name_at_offset(&base_type_2, &bv, 0x0).unwrap(),
            "a".to_string()
        );
        assert_eq!(
            arr_type_a,
            get_member_at_struct_offset(&base_type_2, &bv, 0x0).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&base_type_2, &bv, 0xa8).unwrap(),
            "b".to_string()
        );
        assert_eq!(
            arr_type_b,
            get_member_at_struct_offset(&base_type_2, &bv, 0xa8).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&base_type_2, &bv, 0xc8).unwrap(),
            "c".to_string()
        );
        assert_eq!(
            arr_type_c,
            get_member_at_struct_offset(&base_type_2, &bv, 0xc8).unwrap(),
        );
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
        )
        .unwrap();
        assert!(global_struct.define(bv.as_ref()).is_ok());

        let mut qqq_struct = Structure::new(
            "struct aaa",
            "uint32_t a;\nvoid* b;",
            bv.as_ref(),
            vec!["QQQ".to_string()],
        )
        .unwrap();
        assert!(qqq_struct.define(bv.as_ref()).is_ok());

        let mut rrr_struct = Structure::new(
            "struct aaa",
            "void* a;\nuint32_t b;\nuint64_t c;",
            bv.as_ref(),
            vec!["QQQ".to_string(), "RRR".to_string()],
        )
        .unwrap();
        assert!(rrr_struct.define(bv.as_ref()).is_ok());

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
            Structure::new("struct TestType", "int32_t x;", bv.as_ref(), Vec::new()).unwrap();
        assert!(global_struct.define(bv.as_ref()).is_ok());

        let mut ns_struct = Structure::new(
            "struct TestType",
            "uint32_t y;",
            bv.as_ref(),
            vec!["NS".to_string()],
        )
        .unwrap();
        assert!(ns_struct.define(bv.as_ref()).is_ok());

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
        )
        .unwrap();
        assert!(ns_struct.define(bv.as_ref()).is_ok());

        // Create a member that references this type from within the same namespace
        let member = Member::new(
            "MyType member_field",
            bv.as_ref(),
            None,
            None,
            &vec!["TestNS".to_string()],
        )
        .unwrap();

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
        let global_enum =
            Enum::new("GlobalEnum", 4, "VALUE1,\nVALUE2,\nVALUE3", Vec::new()).unwrap();
        assert!(global_enum.define(bv.as_ref()).is_ok());

        let ns_enum = Enum::new(
            "GlobalEnum",
            4,
            "NS_VALUE1,\nNS_VALUE2",
            vec!["MyNS".to_string()],
        )
        .unwrap();
        assert!(ns_enum.define(bv.as_ref()).is_ok());

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
        if let Err(e) = global_template.define(vec!["int32_t".to_string()], bv.as_ref()) {
            println!("Could not define global template {e}");
            assert!(false);
        }
        if let Err(e) = ns_template.define(vec!["int32_t".to_string()], bv.as_ref()) {
            println!("Could not define ns template {e}");
            assert!(false);
        }

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
        let mut global_class = Class::new(
            "class MyClass",
            "MyClass();\n~MyClass();\n// ; end vtable\nint32_t value;",
            bv.as_ref(),
            Vec::new(),
        )
        .unwrap();
        assert!(global_class.define(bv.as_ref()).is_ok());

        let mut ns_class = Class::new(
            "class MyClass",
            "MyClass();\n~MyClass();\n// ; end vtable\nint64_t data;",
            bv.as_ref(),
            vec!["Services".to_string()],
        )
        .unwrap();
        assert!(ns_class.define(bv.as_ref()).is_ok());

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
        if let Err(e) = parser.parse(namespace_code) {
            println!("Could not parse input code {e}");
            assert!(false);
        }

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

    #[test]
    fn test_multi_level_inheritance_vtable_overrides() {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("test.bndb");
        let headless_session = Session::new().expect("Failed to initialize session");
        let bv = headless_session.load(&path).expect("Couldn't open bv");

        // Define base class with virtual methods
        let mut base_class = Class::new(
            "class BaseClass",
            r#"BaseClass();
~BaseClass();
void methodA();
int32_t methodB(int32_t param);
// ; end vtable
int32_t base_member;"#,
            bv.as_ref(),
            Vec::new(),
        )
        .unwrap();
        assert!(base_class.define(bv.as_ref()).is_ok());

        // Define middle class that inherits from BaseClass and overrides some methods
        let mut middle_class = Class::new(
            "class MiddleClass : BaseClass",
            r#"MiddleClass(); // ; override void (* BaseClass_vtable::BaseClass)(struct BaseClass* this);
~MiddleClass(); // ; override void (* BaseClass_vtable::~BaseClass)(struct BaseClass* this);
int32_t methodB(int32_t param); // ; override int32_t (* BaseClass_vtable::methodB)(struct BaseClass* this, int32_t param);
void methodC();
// ; end vtable
int64_t middle_member;"#,
            bv.as_ref(),
            Vec::new(),
        )
        .unwrap();
        assert!(middle_class.define(bv.as_ref()).is_ok());

        // Define derived class that inherits from MiddleClass and overrides more methods
        let mut derived_class = Class::new(
            "class DerivedClass : MiddleClass",
            r#"DerivedClass(); // ; override void (* MiddleClass_vtable_BaseClass::MiddleClass)(struct MiddleClass* this);
~DerivedClass(); // ; override void (* MiddleClass_vtable_BaseClass::~MiddleClass)(struct MiddleClass* this);
void methodA(); // ; override void (* BaseClass_vtable::methodA)(struct BaseClass* this);
void methodC(); // ; override void (* MiddleClass_vtable_BaseClass::methodC)(struct MiddleClass* this);
bool methodD(float f);
// ; end vtable
uint32_t derived_member;"#,
            bv.as_ref(),
            Vec::new(),
        )
        .unwrap();
        assert!(derived_class.define(bv.as_ref()).is_ok());

        // Verify all classes are defined
        assert!(bv.type_id_by_name("BaseClass").is_some());
        assert!(bv.type_id_by_name("MiddleClass").is_some());
        assert!(bv.type_id_by_name("DerivedClass").is_some());

        // Check that vtable definition worked correctly
        assert_eq!(
            get_type_width_by_name("BaseClass_vtable", &bv).unwrap(),
            0x20
        );
        assert_eq!(
            get_type_width_by_name("MiddleClass_vtable_BaseClass", &bv).unwrap(),
            0x28
        );
        assert_eq!(
            get_type_width_by_name("DerivedClass_vtable_MiddleClass", &bv).unwrap(),
            0x30
        );

        // Verify inheritance chain
        assert_eq!(base_class.base_classes.len(), 0);
        assert_eq!(middle_class.base_classes.len(), 1);
        assert_eq!(middle_class.base_classes[0], "BaseClass");
        assert_eq!(derived_class.base_classes.len(), 1);
        assert_eq!(derived_class.base_classes[0], "MiddleClass");

        // Check arguments to inherited and derived functions
        let base_type = get_type_by_name("BaseClass_vtable", &bv).unwrap();
        let middle_type = get_type_by_name("MiddleClass_vtable_BaseClass", &bv).unwrap();
        let derived_type = get_type_by_name("DerivedClass_vtable_MiddleClass", &bv).unwrap();
        let base_class_ptr = Type::pointer(
            &bv.default_arch().expect("Could not find default arch"),
            &get_non_primitive_type_by_name("BaseClass", &bv).unwrap(),
        );
        let middle_class_ptr = Type::pointer(
            &bv.default_arch().expect("Could not find default arch"),
            &get_non_primitive_type_by_name("MiddleClass", &bv).unwrap(),
        );
        let derived_class_ptr = Type::pointer(
            &bv.default_arch().expect("Could not find default arch"),
            &get_non_primitive_type_by_name("DerivedClass", &bv).unwrap(),
        );

        // BaseClass argument is type BaseClass
        assert_eq!(
            get_member_name_at_offset(&base_type, &bv, 0x0).unwrap(),
            "BaseClass".to_string()
        );
        assert_eq!(
            base_class_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&base_type, &bv, 0x0).unwrap(),
                "this",
            )
            .unwrap()
        );
        // ~BaseClass argument is type BaseClass
        assert_eq!(
            get_member_name_at_offset(&base_type, &bv, 0x8).unwrap(),
            "~BaseClass".to_string()
        );
        assert_eq!(
            base_class_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&base_type, &bv, 0x8).unwrap(),
                "this",
            )
            .unwrap()
        );
        // methodA argument is type BaseClass
        assert_eq!(
            get_member_name_at_offset(&base_type, &bv, 0x10).unwrap(),
            "methodA".to_string()
        );
        assert_eq!(
            base_class_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&base_type, &bv, 0x10).unwrap(),
                "this",
            )
            .unwrap()
        );
        // methodB argument is type BaseClass
        assert_eq!(
            get_member_name_at_offset(&base_type, &bv, 0x18).unwrap(),
            "methodB".to_string()
        );
        assert_eq!(
            base_class_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&base_type, &bv, 0x18).unwrap(),
                "this",
            )
            .unwrap()
        );

        // MiddleClass argument is type MiddleClass
        assert_eq!(
            get_member_name_at_offset(&middle_type, &bv, 0x0).unwrap(),
            "MiddleClass".to_string()
        );
        assert_eq!(
            middle_class_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&middle_type, &bv, 0x0).unwrap(),
                "this",
            )
            .unwrap()
        );
        // ~MiddleClass argument is type MiddleClass
        assert_eq!(
            get_member_name_at_offset(&middle_type, &bv, 0x8).unwrap(),
            "~MiddleClass".to_string()
        );
        assert_eq!(
            middle_class_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&middle_type, &bv, 0x8).unwrap(),
                "this",
            )
            .unwrap()
        );
        // methodA argument is type BaseClass - not overridden
        assert_eq!(
            get_member_name_at_offset(&middle_type, &bv, 0x10).unwrap(),
            "methodA".to_string()
        );
        assert_eq!(
            base_class_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&middle_type, &bv, 0x10).unwrap(),
                "this",
            )
            .unwrap()
        );
        // methodB argument is type MiddleClass
        assert_eq!(
            get_member_name_at_offset(&middle_type, &bv, 0x18).unwrap(),
            "methodB".to_string()
        );
        assert_eq!(
            middle_class_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&middle_type, &bv, 0x18).unwrap(),
                "this",
            )
            .unwrap()
        );
        // methodC argument is type MiddleClass
        assert_eq!(
            get_member_name_at_offset(&middle_type, &bv, 0x20).unwrap(),
            "methodC".to_string()
        );
        assert_eq!(
            middle_class_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&middle_type, &bv, 0x20).unwrap(),
                "this",
            )
            .unwrap()
        );

        // DerivedClass argument is type DerivedClass
        assert_eq!(
            get_member_name_at_offset(&derived_type, &bv, 0x0).unwrap(),
            "DerivedClass".to_string()
        );
        assert_eq!(
            derived_class_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&derived_type, &bv, 0x0).unwrap(),
                "this",
            )
            .unwrap()
        );
        // ~DerivedClass argument is type DerivedClass
        assert_eq!(
            get_member_name_at_offset(&derived_type, &bv, 0x8).unwrap(),
            "~DerivedClass".to_string()
        );
        assert_eq!(
            derived_class_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&derived_type, &bv, 0x8).unwrap(),
                "this",
            )
            .unwrap()
        );
        // methodA argument is type DerivedClass - overridden at this level
        assert_eq!(
            get_member_name_at_offset(&derived_type, &bv, 0x10).unwrap(),
            "methodA".to_string()
        );
        assert_eq!(
            derived_class_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&derived_type, &bv, 0x10).unwrap(),
                "this",
            )
            .unwrap()
        );
        // methodB argument is type MiddleClass - Not overridden
        assert_eq!(
            get_member_name_at_offset(&derived_type, &bv, 0x18).unwrap(),
            "methodB".to_string()
        );
        assert_eq!(
            middle_class_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&derived_type, &bv, 0x18).unwrap(),
                "this",
            )
            .unwrap()
        );
        // methodC argument is type DerivedClass
        assert_eq!(
            get_member_name_at_offset(&derived_type, &bv, 0x20).unwrap(),
            "methodC".to_string()
        );
        assert_eq!(
            derived_class_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&derived_type, &bv, 0x20).unwrap(),
                "this",
            )
            .unwrap()
        );
    }

    #[test]
    fn test_multi_level_inheritance_member_handling() {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("test.bndb");
        let headless_session = Session::new().expect("Failed to initialize session");
        let bv = headless_session.load(&path).expect("Couldn't open bv");

        // Define base class with member
        let mut base_class = Class::new(
            "class MemberBase",
            r#"MemberBase();
~MemberBase();
// ; end vtable
char base_char;
int32_t base_int;"#,
            bv.as_ref(),
            Vec::new(),
        )
        .unwrap();
        assert!(base_class.define(bv.as_ref()).is_ok());

        // Define middle class with additional members and member override
        let mut middle_class = Class::new(
            "class MemberMiddle : MemberBase",
            r#"MemberMiddle();
~MemberMiddle();
// ; end vtable
int64_t middle_long;
void* middle_ptr;
uint32_t base_int; // ; override int32_t MemberBase::base_int;"#,
            bv.as_ref(),
            Vec::new(),
        )
        .unwrap();
        assert!(middle_class.define(bv.as_ref()).is_ok());

        // Define derived class with more members and another override
        let mut derived_class = Class::new(
            "class MemberDerived : MemberMiddle",
            r#"MemberDerived();
~MemberDerived();
// ; end vtable
uint16_t derived_short;
bool derived_bool;
bool base_char; // ; override char MemberBase::base_char;
uint64_t middle_ptr; // ; override void* MemberMiddle::middle_ptr;"#,
            bv.as_ref(),
            Vec::new(),
        )
        .unwrap();
        assert!(derived_class.define(bv.as_ref()).is_ok());

        // Verify class sizes include inherited members
        let base_size = get_type_width_by_name("MemberBase", &bv).expect("Base class size");
        let middle_size = get_type_width_by_name("MemberMiddle", &bv).expect("Middle class size");
        let derived_size =
            get_type_width_by_name("MemberDerived", &bv).expect("Derived class size");

        // Base class: vtable ptr (8) + char (1) + padding (3) + int32_t (4) = 16 bytes
        assert_eq!(base_size, 0x10);

        // Middle class: Base class (16) + long (8) + pointer (8)
        assert_eq!(middle_size, 0x20);

        // Derived class: Middle class (0x20) + short (2) + bool (1) + padding (1)
        assert_eq!(derived_size, 0x24);

        // Verify member counts
        assert_eq!(base_class.member_variables.len(), 2); // base_char, base_int
        assert_eq!(middle_class.member_variables.len(), 3); // middle_long, middle_ptr, base_int override
        assert_eq!(derived_class.member_variables.len(), 4); // derived_short, derived_bool, middle_ptr override, base_char override

        // Verify override details
        if let Some((Member::Basic { name, typ, .. }, _)) =
            base_class.member_variables.last().as_ref()
        {
            assert_eq!(*typ, is_primitive("int32_t").unwrap());
            assert_eq!(name, "base_int");
        } else {
            panic!("Wrong type for member variable");
        }
        if let Some((Member::Basic { name, typ, .. }, _)) =
            middle_class.member_variables.last().as_ref()
        {
            assert_eq!(*typ, is_primitive("uint32_t").unwrap());
            assert_eq!(name, "base_int");
        } else {
            panic!("Wrong type for member variable");
        }

        if let Some((Member::Basic { name, typ, .. }, _)) =
            derived_class.member_variables.last().as_ref()
        {
            assert_eq!(*typ, is_primitive("uint64_t").unwrap());
            assert_eq!(name, "middle_ptr");
        } else {
            panic!("Wrong type for member variable");
        }

        // Verify class composition
        let base_type = get_type_by_name("MemberBase", &bv).unwrap();
        let middle_type = get_type_by_name("MemberMiddle", &bv).unwrap();
        let derived_type = get_type_by_name("MemberDerived", &bv).unwrap();
        let base_class_vtable_ptr = Type::pointer(
            &bv.default_arch().expect("Could not find default arch"),
            &get_non_primitive_type_by_name("MemberBase_vtable", &bv).unwrap(),
        );
        let middle_class_vtable_ptr = Type::pointer(
            &bv.default_arch().expect("Could not find default arch"),
            &get_non_primitive_type_by_name("MemberMiddle_vtable_MemberBase", &bv).unwrap(),
        );
        let derived_class_vtable_ptr = Type::pointer(
            &bv.default_arch().expect("Could not find default arch"),
            &get_non_primitive_type_by_name("MemberDerived_vtable_MemberMiddle", &bv).unwrap(),
        );
        let void_ptr = Type::pointer(
            &bv.default_arch().expect("Could not find default arch"),
            &is_primitive("void").unwrap(),
        );

        // MemberBase should be
        // 0x0: MemberBase_vtable* vtable
        // 0x8: char base_char
        // 0xc: int32_t base_int
        assert_eq!(
            get_member_name_at_offset(&base_type, &bv, 0x0).unwrap(),
            "vtable".to_string()
        );
        assert_eq!(
            base_class_vtable_ptr,
            get_member_at_struct_offset(&base_type, &bv, 0x0).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&base_type, &bv, 0x8).unwrap(),
            "base_char".to_string()
        );
        assert_eq!(
            is_primitive("char").unwrap(),
            get_member_at_struct_offset(&base_type, &bv, 0x8).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&base_type, &bv, 0xc).unwrap(),
            "base_int".to_string()
        );
        assert_eq!(
            is_primitive("int32_t").unwrap(),
            get_member_at_struct_offset(&base_type, &bv, 0xc).unwrap(),
        );

        // MemberMiddle should be
        // 0x0: MemberMiddle_vtable_MemberBase* vtable_MemberBase
        // 0x8: char base_char
        // 0xc: uint32_t base_int // overridden
        // 0x10: int64_t middle_long
        // 0x18: void* middle_ptr
        assert_eq!(
            get_member_name_at_offset(&middle_type, &bv, 0x0).unwrap(),
            "vtable_MemberBase".to_string()
        );
        assert_eq!(
            middle_class_vtable_ptr,
            get_member_at_struct_offset(&middle_type, &bv, 0x0).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&middle_type, &bv, 0x8).unwrap(),
            "base_char".to_string()
        );
        assert_eq!(
            is_primitive("char").unwrap(),
            get_member_at_struct_offset(&middle_type, &bv, 0x8).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&middle_type, &bv, 0xc).unwrap(),
            "base_int".to_string()
        );
        assert_eq!(
            is_primitive("uint32_t").unwrap(),
            get_member_at_struct_offset(&middle_type, &bv, 0xc).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&middle_type, &bv, 0x10).unwrap(),
            "middle_long".to_string()
        );
        assert_eq!(
            is_primitive("int64_t").unwrap(),
            get_member_at_struct_offset(&middle_type, &bv, 0x10).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&middle_type, &bv, 0x18).unwrap(),
            "middle_ptr".to_string()
        );
        assert_eq!(
            void_ptr,
            get_member_at_struct_offset(&middle_type, &bv, 0x18).unwrap(),
        );

        // MemberDerived should be
        // 0x0: MemberDerived_vtable_MemberMiddle* vtable_MemberMiddle
        // 0x8: bool base_char
        // 0xc: uint32_t base_int // overridden in Middle
        // 0x10: int64_t middle_long
        // 0x18: uint64_t middle_ptr
        // 0x20: uint16_t derived_short
        // 0x22: bool derived_bool
        assert_eq!(
            get_member_name_at_offset(&derived_type, &bv, 0x0).unwrap(),
            "vtable_MemberMiddle".to_string()
        );
        assert_eq!(
            derived_class_vtable_ptr,
            get_member_at_struct_offset(&derived_type, &bv, 0x0).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&derived_type, &bv, 0x8).unwrap(),
            "base_char".to_string()
        );
        assert_eq!(
            is_primitive("bool").unwrap(),
            get_member_at_struct_offset(&derived_type, &bv, 0x8).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&derived_type, &bv, 0xc).unwrap(),
            "base_int".to_string()
        );
        assert_eq!(
            is_primitive("uint32_t").unwrap(),
            get_member_at_struct_offset(&derived_type, &bv, 0xc).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&derived_type, &bv, 0x10).unwrap(),
            "middle_long".to_string()
        );
        assert_eq!(
            is_primitive("int64_t").unwrap(),
            get_member_at_struct_offset(&derived_type, &bv, 0x10).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&derived_type, &bv, 0x18).unwrap(),
            "middle_ptr".to_string()
        );
        assert_eq!(
            is_primitive("uint64_t").unwrap(),
            get_member_at_struct_offset(&derived_type, &bv, 0x18).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&derived_type, &bv, 0x20).unwrap(),
            "derived_short".to_string()
        );
        assert_eq!(
            is_primitive("uint16_t").unwrap(),
            get_member_at_struct_offset(&derived_type, &bv, 0x20).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&derived_type, &bv, 0x22).unwrap(),
            "derived_bool".to_string()
        );
        assert_eq!(
            is_primitive("bool").unwrap(),
            get_member_at_struct_offset(&derived_type, &bv, 0x22).unwrap(),
        );
    }

    #[test]
    fn test_multi_level_multiple_inheritance() {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("test.bndb");
        let headless_session = Session::new().expect("Failed to initialize session");
        let bv = headless_session.load(&path).expect("Couldn't open bv");

        // Define four base classes
        let mut base1 = Class::new(
            "class Base1",
            r#"Base1();
~Base1();
void method1();
// ; end vtable
int32_t base1_member;
int32_t base1_member2"#,
            bv.as_ref(),
            Vec::new(),
        )
        .unwrap();
        assert!(base1.define(bv.as_ref()).is_ok());

        let mut base2 = Class::new(
            "class Base2",
            r#"Base2();
~Base2();
void method2();
// ; end vtable
int64_t base2_member;"#,
            bv.as_ref(),
            Vec::new(),
        )
        .unwrap();
        assert!(base2.define(bv.as_ref()).is_ok());

        let mut base3 = Class::new(
            "class Base3",
            r#"Base3();
~Base3();
void method3();
// ; end vtable
uint32_t base3_member;
uint32_t base3_member2"#,
            bv.as_ref(),
            Vec::new(),
        )
        .unwrap();
        assert!(base3.define(bv.as_ref()).is_ok());

        let mut base4 = Class::new(
            "class Base4",
            r#"Base4();
~Base4();
void method4();
// ; end vtable
uint64_t base4_member;"#,
            bv.as_ref(),
            Vec::new(),
        )
        .unwrap();
        assert!(base4.define(bv.as_ref()).is_ok());

        // Define middle class with multiple inheritance (using test.hpp syntax)
        let mut middle1 = Class::new(
            "class MultiMiddle1 : Base1, Base2",
            r#"MultiMiddle1(); // ; override void (* Base1_vtable::Base1)(struct Base1* this);
~MultiMiddle1(); // ; override void (* Base1_vtable::~Base1)(struct Base1* this);
void MultiMiddle1_constructor(); // ; override void (* Base2_vtable::Base2)(struct Base2* this);
void MultiMiddle1_destructor(); // ; override void (* Base2_vtable::~Base2)(struct Base2* this);
void method1(); // ; override void (* Base1_vtable::method1)(struct Base1* this);
void methodMiddle1();
// ; end vtable
uint64_t base2_member; // ; override int64_t Base2::base2_member;
uint64_t middle1_member;"#,
            bv.as_ref(),
            Vec::new(),
        )
        .unwrap();
        assert!(middle1.define(bv.as_ref()).is_ok());

        let mut middle2 = Class::new(
            "class MultiMiddle2 : Base3, Base4",
            r#"MultiMiddle2(); // ; override void (* Base3_vtable::Base3)(struct Base3* this);
~MultiMiddle2(); // ; override void (* Base3_vtable::~Base3)(struct Base3* this);
void MultiMiddle2_constructor(); // ; override void (* Base4_vtable::Base4)(struct Base4* this);
void MultiMiddle2_destructor(); // ; override void (* Base4_vtable::~Base4)(struct Base4* this);
void method4(); // ; override void (* Base4_vtable::method4)(struct Base4* this);
void methodMiddle2();
// ; end vtable
int64_t middle2_member;"#,
            bv.as_ref(),
            Vec::new(),
        )
        .unwrap();
        assert!(middle2.define(bv.as_ref()).is_ok());

        // Define derived class inheriting from multiple inheritance middle class
        let mut derived = Class::new(
            "class MultiDerived : MultiMiddle1, MultiMiddle2",
            r#"MultiDerived(); // ; override void (* MultiMiddle1_vtable_Base1::MultiMiddle1)(struct MultiMiddle1* this);
~MultiDerived(); // ; override void (* MultiMiddle1_vtable_Base1::~MultiMiddle1)(struct MultiMiddle1* this);
void MultiDerived_constructor1(); // ; override void (* MultiMiddle1_vtable_Base2::MultiMiddle1_constructor)(struct MultiMiddle1* this);
void MultiDerived_destructor1(); // ; override void (* MultiMiddle1_vtable_Base2::MultiMiddle1_destructor)(struct MultiMiddle1* this);
void MultiDerived_constructor2(); // ; override void (* MultiMiddle2_vtable_Base3::MultiMiddle2)(struct MultiMiddle2* this);
void MultiDerived_destructor2(); // ; override void (* MultiMiddle2_vtable_Base3::~MultiMiddle2)(struct MultiMiddle2* this);
void MultiDerived_constructor3(); // ; override void (* MultiMiddle2_vtable_Base4::MultiMiddle2_constructor)(struct MultiMiddle2* this);
void MultiDerived_destructor3(); // ; override void (* MultiMiddle2_vtable_Base4::MultiMiddle2_destructor)(struct MultiMiddle2* this);
void methodMiddle1(); // ; override void (* MultiMiddle1_vtable_Base1::methodMiddle1)(struct MultiMiddle1* this);
void method4(); // ; override void (* Base4_vtable::method4)(struct Base4* this);
void methodDerived();
// ; end vtable
int64_t base4_member; // ; override uint64_t Base4::base4_member;
bool derived_member;"#,
            bv.as_ref(),
            Vec::new(),
        )
        .unwrap();
        assert!(derived.define(bv.as_ref()).is_ok());

        // Verify inheritance relationships
        assert_eq!(middle1.base_classes.len(), 2);
        assert!(middle1.base_classes.contains(&"Base1".to_string()));
        assert!(middle1.base_classes.contains(&"Base2".to_string()));
        assert_eq!(middle2.base_classes.len(), 2);
        assert!(middle2.base_classes.contains(&"Base3".to_string()));
        assert!(middle2.base_classes.contains(&"Base4".to_string()));
        assert_eq!(derived.base_classes.len(), 2);
        assert!(derived.base_classes.contains(&"MultiMiddle1".to_string()));
        assert!(derived.base_classes.contains(&"MultiMiddle2".to_string()));

        // Verify vtable methods are collected properly
        assert_eq!(base1.vtable_methods.len(), 0); // constructor, destructor, method1
        assert_eq!(base2.vtable_methods.len(), 0); // constructor, destructor, method2
        assert_eq!(base3.vtable_methods.len(), 0); // constructor, destructor, method1
        assert_eq!(base4.vtable_methods.len(), 0); // constructor, destructor, method2
        assert_eq!(middle1.vtable_methods.len(), 0); // overrides + new methodMiddle
        assert_eq!(middle2.vtable_methods.len(), 0); // overrides + new methodMiddle
        assert_eq!(derived.vtable_methods.len(), 0); // overrides + new methodDerived

        // Check sizes account for multiple inheritance
        assert_eq!(get_type_width_by_name("Base1", &bv).unwrap(), 0x10); // vtable (8) + int32_t * 2 (8)
        assert_eq!(get_type_width_by_name("Base2", &bv).unwrap(), 0x10); // vtable (8) + int64_t (8)
        assert_eq!(get_type_width_by_name("Base3", &bv).unwrap(), 0x10); // vtable (8) + uint32_t * 2 (8)
        assert_eq!(get_type_width_by_name("Base4", &bv).unwrap(), 0x10); // vtable (8) + uint64_t (8)
        assert_eq!(get_type_width_by_name("MultiMiddle1", &bv).unwrap(), 0x28); // base1 (0x10) + base2 (0x10) + uint64_t (8)
        assert_eq!(get_type_width_by_name("MultiMiddle2", &bv).unwrap(), 0x28); // base3 (0x10) + base4 (0x10) + uint64_t (8)
        assert_eq!(get_type_width_by_name("MultiDerived", &bv).unwrap(), 0x51); // MultiMiddle1 (0x28) + MultiMiddle2 (0x28) + bool (1) + padding (3)

        // Verify class composition
        let derived_type = get_type_by_name("MultiDerived", &bv).unwrap();
        let derived_type_vtable_1 =
            get_type_by_name("MultiDerived_vtable_MultiMiddle1", &bv).unwrap();
        let derived_type_vtable_2 = get_type_by_name("MultiDerived_vtable_Base2", &bv).unwrap();
        let derived_type_vtable_3 =
            get_type_by_name("MultiDerived_vtable_MultiMiddle2", &bv).unwrap();
        let derived_type_vtable_4 = get_type_by_name("MultiDerived_vtable_Base4", &bv).unwrap();
        let base_2_class_ptr = Type::pointer(
            &bv.default_arch().expect("Could not find default arch"),
            &get_non_primitive_type_by_name("Base2", &bv).unwrap(),
        );
        let base_3_class_ptr = Type::pointer(
            &bv.default_arch().expect("Could not find default arch"),
            &get_non_primitive_type_by_name("Base3", &bv).unwrap(),
        );
        let middle_1_class_ptr = Type::pointer(
            &bv.default_arch().expect("Could not find default arch"),
            &get_non_primitive_type_by_name("MultiMiddle1", &bv).unwrap(),
        );
        let middle_2_class_ptr = Type::pointer(
            &bv.default_arch().expect("Could not find default arch"),
            &get_non_primitive_type_by_name("MultiMiddle2", &bv).unwrap(),
        );
        let derived_class_ptr = Type::pointer(
            &bv.default_arch().expect("Could not find default arch"),
            &get_non_primitive_type_by_name("MultiDerived", &bv).unwrap(),
        );
        let derived_class_vtable_ptr_1 = Type::pointer(
            &bv.default_arch().expect("Could not find default arch"),
            &get_non_primitive_type_by_name("MultiDerived_vtable_MultiMiddle1", &bv).unwrap(),
        );
        let derived_class_vtable_ptr_2 = Type::pointer(
            &bv.default_arch().expect("Could not find default arch"),
            &get_non_primitive_type_by_name("MultiDerived_vtable_Base2", &bv).unwrap(),
        );
        let derived_class_vtable_ptr_3 = Type::pointer(
            &bv.default_arch().expect("Could not find default arch"),
            &get_non_primitive_type_by_name("MultiDerived_vtable_MultiMiddle2", &bv).unwrap(),
        );
        let derived_class_vtable_ptr_4 = Type::pointer(
            &bv.default_arch().expect("Could not find default arch"),
            &get_non_primitive_type_by_name("MultiDerived_vtable_Base4", &bv).unwrap(),
        );

        // Verify MultiDerived members

        // MultiDerived should be
        // 0x00: MultiMiddle1_vtable_Base1* vtable_MultiMiddle1
        // 0x08: int32_t base1_member
        // 0x0c: int32_t base1_member2
        // 0x10: MultiMiddle1_vtable_Base1* vtable_Base2
        // 0x18: uint64_t base2_member // overridden by MultiMiddle1
        // 0x20: uint64_t middle1_member
        // 0x28: MultiMiddle2_vtable_Base3* vtable_MultiMiddle2
        // 0x30: uint32_t base3_member
        // 0x34: uint32_t base3_member2
        // 0x38: MultiMiddle2_vtable_Base4* vtable_Base4
        // 0x40: int64_t base4_member // overridden by MutliDerived
        // 0x48: int64_t middle1_member
        // 0x50: bool derived_member
        assert_eq!(
            get_member_name_at_offset(&derived_type, &bv, 0x0).unwrap(),
            "vtable_MultiMiddle1".to_string()
        );
        assert_eq!(
            derived_class_vtable_ptr_1,
            get_member_at_struct_offset(&derived_type, &bv, 0x0).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&derived_type, &bv, 0x8).unwrap(),
            "base1_member".to_string()
        );
        assert_eq!(
            is_primitive("int32_t").unwrap(),
            get_member_at_struct_offset(&derived_type, &bv, 0x8).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&derived_type, &bv, 0xc).unwrap(),
            "base1_member2".to_string()
        );
        assert_eq!(
            is_primitive("int32_t").unwrap(),
            get_member_at_struct_offset(&derived_type, &bv, 0xc).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&derived_type, &bv, 0x10).unwrap(),
            "vtable_Base2".to_string()
        );
        assert_eq!(
            derived_class_vtable_ptr_2,
            get_member_at_struct_offset(&derived_type, &bv, 0x10).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&derived_type, &bv, 0x18).unwrap(),
            "base2_member".to_string()
        );
        assert_eq!(
            is_primitive("uint64_t").unwrap(),
            get_member_at_struct_offset(&derived_type, &bv, 0x18).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&derived_type, &bv, 0x20).unwrap(),
            "middle1_member".to_string()
        );
        assert_eq!(
            is_primitive("uint64_t").unwrap(),
            get_member_at_struct_offset(&derived_type, &bv, 0x20).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&derived_type, &bv, 0x28).unwrap(),
            "vtable_MultiMiddle2".to_string()
        );
        assert_eq!(
            derived_class_vtable_ptr_3,
            get_member_at_struct_offset(&derived_type, &bv, 0x28).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&derived_type, &bv, 0x30).unwrap(),
            "base3_member".to_string()
        );
        assert_eq!(
            is_primitive("uint32_t").unwrap(),
            get_member_at_struct_offset(&derived_type, &bv, 0x30).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&derived_type, &bv, 0x34).unwrap(),
            "base3_member2".to_string()
        );
        assert_eq!(
            is_primitive("uint32_t").unwrap(),
            get_member_at_struct_offset(&derived_type, &bv, 0x34).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&derived_type, &bv, 0x38).unwrap(),
            "vtable_Base4".to_string()
        );
        assert_eq!(
            derived_class_vtable_ptr_4,
            get_member_at_struct_offset(&derived_type, &bv, 0x38).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&derived_type, &bv, 0x40).unwrap(),
            "base4_member".to_string()
        );
        assert_eq!(
            is_primitive("int64_t").unwrap(),
            get_member_at_struct_offset(&derived_type, &bv, 0x40).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&derived_type, &bv, 0x48).unwrap(),
            "middle2_member".to_string()
        );
        assert_eq!(
            is_primitive("int64_t").unwrap(),
            get_member_at_struct_offset(&derived_type, &bv, 0x48).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&derived_type, &bv, 0x50).unwrap(),
            "derived_member".to_string()
        );
        assert_eq!(
            is_primitive("bool").unwrap(),
            get_member_at_struct_offset(&derived_type, &bv, 0x50).unwrap(),
        );

        // Verify MultiDerived vtables

        // vtable_MultiMiddle1 should be
        // 0x00: MultiDerived()
        // 0x08: ~MultiDerived()
        // 0x10: void method1(MultiMiddle1* this)
        // 0x18: void methodMiddle1(MultiDerived* this)
        // 0x20: void methodDerived(MultiDerived* this)
        assert_eq!(
            get_member_name_at_offset(&derived_type_vtable_1, &bv, 0x0).unwrap(),
            "MultiDerived".to_string()
        );
        assert_eq!(
            derived_class_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&derived_type_vtable_1, &bv, 0x0).unwrap(),
                "this",
            )
            .unwrap()
        );
        assert_eq!(
            get_member_name_at_offset(&derived_type_vtable_1, &bv, 0x8).unwrap(),
            "~MultiDerived".to_string()
        );
        assert_eq!(
            derived_class_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&derived_type_vtable_1, &bv, 0x8).unwrap(),
                "this",
            )
            .unwrap()
        );
        assert_eq!(
            get_member_name_at_offset(&derived_type_vtable_1, &bv, 0x10).unwrap(),
            "method1".to_string()
        );
        assert_eq!(
            middle_1_class_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&derived_type_vtable_1, &bv, 0x10).unwrap(),
                "this",
            )
            .unwrap()
        );
        assert_eq!(
            get_member_name_at_offset(&derived_type_vtable_1, &bv, 0x18).unwrap(),
            "methodMiddle1".to_string()
        );
        assert_eq!(
            derived_class_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&derived_type_vtable_1, &bv, 0x18).unwrap(),
                "this",
            )
            .unwrap()
        );
        assert_eq!(
            get_member_name_at_offset(&derived_type_vtable_1, &bv, 0x20).unwrap(),
            "methodDerived".to_string()
        );
        assert_eq!(
            derived_class_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&derived_type_vtable_1, &bv, 0x20).unwrap(),
                "this",
            )
            .unwrap()
        );

        // vtable_Base2 should be
        // 0x00: MultiDerived_constructor1()
        // 0x08: MultiDerived_destructor1()
        // 0x10: void method2(Base2* this)
        assert_eq!(
            get_member_name_at_offset(&derived_type_vtable_2, &bv, 0x0).unwrap(),
            "MultiDerived_constructor1".to_string()
        );
        assert_eq!(
            derived_class_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&derived_type_vtable_2, &bv, 0x0).unwrap(),
                "this",
            )
            .unwrap()
        );
        assert_eq!(
            get_member_name_at_offset(&derived_type_vtable_2, &bv, 0x8).unwrap(),
            "MultiDerived_destructor1".to_string()
        );
        assert_eq!(
            derived_class_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&derived_type_vtable_2, &bv, 0x8).unwrap(),
                "this",
            )
            .unwrap()
        );
        assert_eq!(
            get_member_name_at_offset(&derived_type_vtable_2, &bv, 0x10).unwrap(),
            "method2".to_string()
        );
        assert_eq!(
            base_2_class_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&derived_type_vtable_2, &bv, 0x10).unwrap(),
                "this",
            )
            .unwrap()
        );

        // vtable_MultiMiddle2 should be
        // 0x00: MultiDerived_constructor2()
        // 0x08: MultiDerived_destructor2()
        // 0x10: void method3(Base3* this)
        // 0x18: void methodMiddle2(MultiMiddle2* this)
        assert_eq!(
            get_member_name_at_offset(&derived_type_vtable_3, &bv, 0x0).unwrap(),
            "MultiDerived_constructor2".to_string()
        );
        assert_eq!(
            derived_class_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&derived_type_vtable_3, &bv, 0x0).unwrap(),
                "this",
            )
            .unwrap()
        );
        assert_eq!(
            get_member_name_at_offset(&derived_type_vtable_3, &bv, 0x8).unwrap(),
            "MultiDerived_destructor2".to_string()
        );
        assert_eq!(
            derived_class_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&derived_type_vtable_3, &bv, 0x8).unwrap(),
                "this",
            )
            .unwrap()
        );
        assert_eq!(
            get_member_name_at_offset(&derived_type_vtable_3, &bv, 0x10).unwrap(),
            "method3".to_string()
        );
        assert_eq!(
            base_3_class_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&derived_type_vtable_3, &bv, 0x10).unwrap(),
                "this",
            )
            .unwrap()
        );
        assert_eq!(
            get_member_name_at_offset(&derived_type_vtable_3, &bv, 0x18).unwrap(),
            "methodMiddle2".to_string()
        );
        assert_eq!(
            middle_2_class_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&derived_type_vtable_3, &bv, 0x18).unwrap(),
                "this",
            )
            .unwrap()
        );

        // vtable_Base4 should be
        // 0x00: MultiDerived_constructor3()
        // 0x08: MultiDerived_destructor3()
        // 0x10: void method4(MultiDerived* this)
        assert_eq!(
            get_member_name_at_offset(&derived_type_vtable_4, &bv, 0x0).unwrap(),
            "MultiDerived_constructor3".to_string()
        );
        assert_eq!(
            derived_class_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&derived_type_vtable_4, &bv, 0x0).unwrap(),
                "this",
            )
            .unwrap()
        );
        assert_eq!(
            get_member_name_at_offset(&derived_type_vtable_4, &bv, 0x8).unwrap(),
            "MultiDerived_destructor3".to_string()
        );
        assert_eq!(
            derived_class_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&derived_type_vtable_4, &bv, 0x8).unwrap(),
                "this",
            )
            .unwrap()
        );
        assert_eq!(
            get_member_name_at_offset(&derived_type_vtable_4, &bv, 0x10).unwrap(),
            "method4".to_string()
        );
        assert_eq!(
            derived_class_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&derived_type_vtable_4, &bv, 0x10).unwrap(),
                "this",
            )
            .unwrap()
        );
    }

    #[test]
    fn test_deep_inheritance_chain() {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("test.bndb");
        let headless_session = Session::new().expect("Failed to initialize session");
        let bv = headless_session.load(&path).expect("Couldn't open bv");

        // Create a 4-level deep inheritance chain
        let mut level1 = Class::new(
            "class Level1",
            r#"Level1();
~Level1();
void virtualMethod();
// ; end vtable
int8_t level1_data;"#,
            bv.as_ref(),
            Vec::new(),
        )
        .unwrap();
        assert!(level1.define(bv.as_ref()).is_ok());

        let mut level2 = Class::new(
            "class Level2 : Level1",
            r#"Level2(); // ; override void (* Level1_vtable::Level1)(struct Level1* this);
~Level2(); // ; override void (* Level1_vtable::~Level1)(struct Level1* this);
void virtualMethod(); // ; override void (* Level1_vtable::virtualMethod)(struct Level1* this);
void level2Method();
// ; end vtable
int16_t level2_data;"#,
            bv.as_ref(),
            Vec::new(),
        )
        .unwrap();
        assert!(level2.define(bv.as_ref()).is_ok());

        let mut level3 = Class::new(
            "class Level3 : Level2",
            r#"Level3(); // ; override void (* Level2_vtable_Level1::Level2)(struct Level2* this);
~Level3(); // ; override void (* Level2_vtable_Level1::~Level2)(struct Level2* this);
void level2Method(); // ; override void (* Level2_vtable_Level1::level2Method)(struct Level2* this);
void level3Method();
// ; end vtable
int32_t level3_data;"#,
            bv.as_ref(),
            Vec::new(),
        )
        .unwrap();
        assert!(level3.define(bv.as_ref()).is_ok());

        let mut level4 = Class::new(
            "class Level4 : Level3",
            r#"Level4(); // ; override void (* Level3_vtable_Level2::Level3)(struct Level3* this);
~Level4(); // ; override void (* Level3_vtable_Level2::~Level3)(struct Level3* this);
void level3Method(); // ; override void (* Level3_vtable_Level2::level3Method)(struct Level3* this);
void level4Method();
// ; end vtable
int64_t level4_data;"#,
            bv.as_ref(),
            Vec::new(),
        )
        .unwrap();
        assert!(level4.define(bv.as_ref()).is_ok());

        // Verify inheritance chain
        assert_eq!(level1.base_classes.len(), 0);
        assert_eq!(level2.base_classes, vec!["Level1"]);
        assert_eq!(level3.base_classes, vec!["Level2"]);
        assert_eq!(level4.base_classes, vec!["Level3"]);

        // Verify vtable method counts
        assert!(level1.vtable_methods.is_empty());
        assert!(level2.vtable_methods.is_empty());
        assert!(level3.vtable_methods.is_empty());
        assert!(level4.vtable_methods.is_empty());

        // Verify MultiDerived members

        let level_4_type = get_type_by_name("Level4", &bv).unwrap();
        let level_4_vtable = get_type_by_name("Level4_vtable_Level3", &bv).unwrap();
        let level_4_vtable_ptr = Type::pointer(
            &bv.default_arch().expect("Could not find default arch"),
            &get_non_primitive_type_by_name("Level4_vtable_Level3", &bv).unwrap(),
        );
        let level_2_ptr = Type::pointer(
            &bv.default_arch().expect("Could not find default arch"),
            &get_non_primitive_type_by_name("Level2", &bv).unwrap(),
        );
        let level_3_ptr = Type::pointer(
            &bv.default_arch().expect("Could not find default arch"),
            &get_non_primitive_type_by_name("Level3", &bv).unwrap(),
        );
        let level_4_ptr = Type::pointer(
            &bv.default_arch().expect("Could not find default arch"),
            &get_non_primitive_type_by_name("Level4", &bv).unwrap(),
        );

        // Level4 should be
        // 0x00: Level4_vtable_Level3* vtable_Level3
        // 0x08: int8_t level1_data
        // 0x0a: int16_t level2_data
        // 0x0c: int32_t level3_data
        // 0x10: int64_t level4_data
        assert_eq!(
            get_member_name_at_offset(&level_4_type, &bv, 0x0).unwrap(),
            "vtable_Level3".to_string()
        );
        assert_eq!(
            level_4_vtable_ptr,
            get_member_at_struct_offset(&level_4_type, &bv, 0x0).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&level_4_type, &bv, 0x8).unwrap(),
            "level1_data".to_string()
        );
        assert_eq!(
            is_primitive("int8_t").unwrap(),
            get_member_at_struct_offset(&level_4_type, &bv, 0x8).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&level_4_type, &bv, 0xa).unwrap(),
            "level2_data".to_string()
        );
        assert_eq!(
            is_primitive("int16_t").unwrap(),
            get_member_at_struct_offset(&level_4_type, &bv, 0xa).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&level_4_type, &bv, 0xc).unwrap(),
            "level3_data".to_string()
        );
        assert_eq!(
            is_primitive("int32_t").unwrap(),
            get_member_at_struct_offset(&level_4_type, &bv, 0xc).unwrap(),
        );
        assert_eq!(
            get_member_name_at_offset(&level_4_type, &bv, 0x10).unwrap(),
            "level4_data".to_string()
        );
        assert_eq!(
            is_primitive("int64_t").unwrap(),
            get_member_at_struct_offset(&level_4_type, &bv, 0x10).unwrap(),
        );

        // Verify Level4 vtable

        // vtable should be
        // 0x00: Level4()
        // 0x08: ~Level4()
        // 0x10: void virtualMethod(Level2* this)
        // 0x18: void level2Method(Level3* this)
        // 0x20: void level3Method(Level4* this)
        // 0x28: void level4Method(Level4* this)
        assert_eq!(
            get_member_name_at_offset(&level_4_vtable, &bv, 0x0).unwrap(),
            "Level4".to_string()
        );
        assert_eq!(
            level_4_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&level_4_vtable, &bv, 0x0).unwrap(),
                "this",
            )
            .unwrap()
        );
        assert_eq!(
            get_member_name_at_offset(&level_4_vtable, &bv, 0x8).unwrap(),
            "~Level4".to_string()
        );
        assert_eq!(
            level_4_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&level_4_vtable, &bv, 0x8).unwrap(),
                "this",
            )
            .unwrap()
        );
        assert_eq!(
            get_member_name_at_offset(&level_4_vtable, &bv, 0x10).unwrap(),
            "virtualMethod".to_string()
        );
        assert_eq!(
            level_2_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&level_4_vtable, &bv, 0x10).unwrap(),
                "this",
            )
            .unwrap()
        );
        assert_eq!(
            get_member_name_at_offset(&level_4_vtable, &bv, 0x18).unwrap(),
            "level2Method".to_string()
        );
        assert_eq!(
            level_3_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&level_4_vtable, &bv, 0x18).unwrap(),
                "this",
            )
            .unwrap()
        );
        assert_eq!(
            get_member_name_at_offset(&level_4_vtable, &bv, 0x20).unwrap(),
            "level3Method".to_string()
        );
        assert_eq!(
            level_4_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&level_4_vtable, &bv, 0x20).unwrap(),
                "this",
            )
            .unwrap()
        );
        assert_eq!(
            get_member_name_at_offset(&level_4_vtable, &bv, 0x28).unwrap(),
            "level4Method".to_string()
        );
        assert_eq!(
            level_4_ptr,
            get_function_argument_type_by_name(
                &get_member_at_struct_offset(&level_4_vtable, &bv, 0x28).unwrap(),
                "this",
            )
            .unwrap()
        );
    }

    #[test]
    fn test_array_typedef() {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("test.bndb");
        let headless_session = Session::new().expect("Failed to initialize session");
        let bv = headless_session.load(&path).expect("Couldn't open bv");

        // Test parsing array typedef like "typedef int array_name[16]"
        let array_typedef = Typedef::new("int my_array[16]", vec![]);
        assert!(array_typedef.define(bv.as_ref()).is_ok());

        // Verify the typedef was created as an array type
        let typedef_type = get_type_by_name("my_array", &bv).expect("Array typedef should exist");
        assert_eq!(typedef_type.width(), 16 * 4); // 16 integers * 4 bytes each

        // Test hex array size
        let hex_typedef = Typedef::new("char hex_array[0x10]", vec![]);
        assert!(hex_typedef.define(bv.as_ref()).is_ok());

        let hex_type = get_type_by_name("hex_array", &bv).expect("Hex array typedef should exist");
        assert_eq!(hex_type.width(), 0x10); // 16 chars * 1 byte each
    }
}
