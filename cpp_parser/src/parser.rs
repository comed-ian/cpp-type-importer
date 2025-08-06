use crate::{
    find_closing_token, find_next_token, parse_template_definition, parse_template_instantiation,
    parse_typedef_assignment, strip_type_prefix, utils::parse_ptr_offset, Class, Enum, Structure,
    Template, Typedef,
};
use binaryninja::{
    binary_view::{BinaryView, BinaryViewExt},
    types::{StructureBuilder, Type},
};
use regex::Regex;

// Main parser for C++ header files
///
/// This parser processes C++ header content and creates corresponding
/// Binary Ninja types including structs, classes, templates, and enums.
pub struct Parser<'a> {
    /// Binary Ninja binary view reference
    pub bv: &'a BinaryView,
    /// Current namespace stack for tracking nested namespaces
    pub namespace_stack: Vec<String>,
    /// List of current templates, potentially defined in previously
    /// parsed header files
    pub templates: Vec<Template>,
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
            templates: vec![],
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
    pub fn get_current_namespace_path(&self) -> Vec<String> {
        self.namespace_stack.clone()
    }

    /// Enters a new namespace
    pub fn enter_namespace(&mut self, namespace_name: &str) {
        self.namespace_stack.push(namespace_name.to_string());
        log::info!("Entering namespace: {}", self.namespace_stack.join("::"));
    }

    /// Exits the current namespace
    pub fn exit_namespace(&mut self) {
        if let Some(namespace) = self.namespace_stack.pop() {
            log::info!("Exiting namespace: {}", namespace);
        }
    }

    pub fn parse(&mut self, contents: &str) -> Result<(), String> {
        // Remove /* */ comments
        let re = Regex::new(r"(?s)/\*.*?\*/").unwrap();
        let contents = re.replace_all(contents, "").to_string();
        let mut idx = 0usize;
        loop {
            // find next ;, {, <, "
            if let Some((i, c, mut s)) = find_next_token(&contents[idx..]) {
                s = s.trim();
                idx += i + 1;
                log::info!("Handling line: {}, {}, {}", i, s, c);
                // base case
                if i == 0 && c == ';' {
                    continue;
                }
                // structure, template, class definition
                // get closing token and index
                if s.starts_with("#include") {
                    let (i2, _, mut s2) = find_closing_token(&contents[idx..], c)
                        .ok_or("Could not find closing token".to_string())?;
                    s2 = s2.trim();
                    // throw out include statements
                    if c != '<' && c != '"' {
                        return Err(
                            "Could not find opening < or \" in include statement".to_string()
                        );
                    }
                    log::info!("Skipping line: {} {}", s, s2);
                    idx += i2 + 1;
                    continue;
                } else if s.starts_with("#pragma") {
                    let (i2, _, mut s2) = find_closing_token(&contents[idx..], c)
                        .ok_or("Could not find closing token".to_string())?;
                    s2 = s2.trim();
                    // throw out pragma statements
                    if c != '"' {
                        return Err("Could not find \"..\" in pragma statement".to_string());
                    }
                    log::info!("Skipping line: {} {}", s, s2);
                    idx += i2 + 1;
                    continue;
                }
                match c {
                    '/' => {
                        // Start of an EOL comments
                        log::info!("Skipping comment {s}");
                        continue;
                    }
                    '}' => {
                        // Assume this is a namespace closing brace and exit current namespace
                        self.exit_namespace();
                    }
                    '<' => {
                        // template
                        if s.starts_with("template") {
                            let (i2, _, mut s2) = find_closing_token(&contents[idx..], c)
                                .ok_or("Could not find closing token")?;
                            s2 = s2.trim();
                            // template definition, store for later declarations
                            log::info!("Got template type: {}", s2);
                            let typenames = parse_template_definition(s2)
                                .ok_or(&format!("Could not parse template definitions {s2}"))?;
                            let (i3, c3, s3) = find_next_token(&contents[idx + i2 + 1..])
                                .ok_or("Could not find closing token for template definition")?;

                            if s3.trim().starts_with("using ") {
                                // Templated typedef: template <...> using AA = Abc<T, uint32_t>;
                                // Find the closing ';' specifically
                                let (i4, _, s4) =
                                    find_closing_token(&contents[idx + i2 + i3 + 1..], ';').ok_or(
                                        "Could not find closing semicolon for templated typedef",
                                    )?;
                                let full_using_statement = format!("{}{}", s3, s4);
                                log::info!("Got templated typedef: {}", full_using_statement);

                                // Strip "using " from the front and parse the assignment
                                let assignment_string =
                                    full_using_statement.trim().strip_prefix("using ").unwrap();
                                let (typedef_name, template_name, target_params, depth) =
                                    parse_typedef_assignment(assignment_string)?
                                        .ok_or("Could not parse templated typedef assignment")?;

                                // Separate templated parameters from concrete parameters
                                let mut templated_parameters = Vec::new();
                                let mut concrete_parameters = Vec::new();

                                for (idx, param) in target_params.iter().enumerate() {
                                    let param = param.trim();
                                    if typenames.contains(&param.to_string()) {
                                        templated_parameters.push((idx, param.to_string()));
                                    } else {
                                        concrete_parameters.push((idx, param.to_string()));
                                    }
                                }

                                let t = Template::new_typedef(
                                    typedef_name,
                                    typenames.clone(),
                                    template_name,
                                    templated_parameters,
                                    concrete_parameters,
                                    depth,
                                    self.get_current_namespace_path(),
                                );
                                self.templates.push(t);

                                idx += i3 + i4 + 2;
                            } else if c3 == '{' {
                                // Regular struct/class template
                                log::info!("Got template name: {}", s3);
                                let (i4, _, body) =
                                    find_closing_token(&contents[idx + i2 + i3 + 2..], c3)
                                        .ok_or("Could not find closing token")?;
                                log::info!("Got template definition: {}", body);
                                idx += i3 + i4 + 2;
                                let t = Template::new(
                                    s3,
                                    body,
                                    typenames,
                                    self.get_current_namespace_path(),
                                );
                                self.templates.push(t);
                            } else {
                                // Unknown template pattern, skip it
                                return Err(format!(
                                    "Unknown template pattern: s3='{}', c3='{}'",
                                    s3, c3
                                ));
                            }
                            idx += i2 + 1;
                        } else {
                            // s looks like `struct_name<` or `typedef struct_name<`.
                            // Find closing `;` to get the contents within the angled brackets.
                            let (i2, _, mut s2) = find_closing_token(&contents[idx..], ';')
                                .ok_or("Could not find closing token for template instantiation")?;
                            if s.starts_with("typedef ") {
                                let mut typedef_string = s[8..].to_string();
                                typedef_string.push('<');
                                typedef_string.push_str(&s2);
                                log::info!("Got typedef string: {}", typedef_string);
                                let t = Typedef::new(
                                    &typedef_string,
                                    self.get_current_namespace_path(),
                                );
                                t.define(self.bv)?;
                            } else {
                                log::info!("Handling line: {} {}", s, s2);
                                if let Some(stripped) = s2.strip_suffix('>') {
                                    s2 = stripped;
                                }
                                log::info!("Got template instantiation: {}", s2);
                                let typenames = parse_template_instantiation(s2)?
                                    .ok_or(format!("Could not parse template definitions {s2}"))?;
                                // check for template named `s` to declare
                                let t = self
                                    .templates
                                    .iter()
                                    .find(|x| {
                                        x.get_name() == s.trim() || x.get_full_name() == s.trim()
                                    })
                                    .ok_or(&format!(
                                        "Could not find template {s} for definition"
                                    ))?;
                                // Peek ahead to check for possible __ptr_offset
                                let off = if let Some((_, _, peek)) =
                                    find_next_token(&contents[idx + i2 + 1..])
                                {
                                    parse_ptr_offset(peek)?.unwrap_or(0)
                                } else {
                                    0
                                };

                                t.define(
                                    typenames,
                                    self.bv,
                                    off,
                                    &self.get_current_namespace_path(),
                                )?;
                            }
                            idx += i2 + 1;
                        }
                    }
                    ';' => {
                        if s.starts_with("typedef ") {
                            // typedef like `typedef struct_1 struct_2;`. Templated types
                            // match to `<` and are handled above.
                            let t = Typedef::new(&s[8..], self.get_current_namespace_path());
                            t.define(self.bv)?;
                        } else {
                            // forward declaration
                            let name = strip_type_prefix(s)
                                .ok_or("Error stripping prefix for forward declaration")?
                                .trim();
                            let current_namespaces = self.get_current_namespace_path();
                            let forward_decl = Type::structure(&StructureBuilder::new().finalize());
                            let full_name = if current_namespaces.is_empty() {
                                name.to_string()
                            } else {
                                format!("{}::{}", current_namespaces.join("::"), name)
                            };
                            log::info!("Forward declaring {full_name}");
                            self.bv.define_user_type(&full_name, &forward_decl);
                        }
                    }
                    '{' => {
                        // namespace, class, or struct definition
                        if s.starts_with("namespace ") {
                            // Extract namespace name
                            let namespace_name = s.strip_prefix("namespace ").unwrap().trim();
                            if !namespace_name.is_empty() {
                                self.enter_namespace(namespace_name);
                                // Continue parsing inside the namespace
                            }
                        } else if s.starts_with("struct") {
                            let (i2, _, mut s2) = find_closing_token(&contents[idx..], c)
                                .ok_or("Could not find closing token")?;
                            s2 = s2.trim();
                            log::info!("Got struct {}: {}", s, s2);
                            let mut structure =
                                Structure::new(s, s2, self.bv, self.get_current_namespace_path())?;
                            structure.define(self.bv)?;
                            idx += i2 + 1;
                        } else if s.starts_with("class") {
                            let (i2, _, mut s2) = find_closing_token(&contents[idx..], c)
                                .ok_or("Could not find closing token")?;
                            s2 = s2.trim();
                            log::debug!("Got class {}: {}", s, s2);
                            let mut class =
                                Class::new(s, s2, self.bv, self.get_current_namespace_path())?;
                            class.define(self.bv)?;
                            idx += i2 + 1;
                        } else if s.starts_with("enum") {
                            let (i2, _, mut s2) = find_closing_token(&contents[idx..], c)
                                .ok_or("Could not find closing token")?;
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
                                Enum::new(enum_name, size, s2, self.get_current_namespace_path())?;
                            enum_def.define(self.bv)?;

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
        Ok(())
    }
}
