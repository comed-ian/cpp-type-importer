use binaryninja::binary_view::{BinaryView, BinaryViewExt};
use binaryninja::types::{StructureBuilder, Type};

use crate::utils::resolve_type_name;
use crate::{Member, Structure, get_non_primitive_type_by_name, parse_name};

/// Represents different types of C++ templates
#[derive(Debug, Clone)]
pub enum Template {
    /// Regular template with a body (struct/class template)
    StructTemplate {
        /// The name of the template (e.g., "vector" for std::vector)
        name: String,
        /// List of template parameter names (e.g., ["T", "Allocator"])
        typenames: Vec<String>,
        /// The template body containing member definitions
        body: String,
        /// Namespace path for this template
        namespace_path: Vec<String>,
    },
    /// Templated typedef (using statement) like `template <typename T> using AA = Abc<T, uint32_t>;`
    TypedefTemplate {
        /// The name of the templated typedef (e.g., "AA")
        name: String,
        /// List of template parameter names (e.g., ["T"])
        typenames: Vec<String>,
        /// The base template name being aliased (e.g., "Abc")
        target_template_name: String,
        /// Templated parameters: (target_index, template_param_name)
        /// e.g., for Abc<T, uint32_t> where T is templated: [(0, "T")]
        /// e.g., for Abc<uint32_t, T> where T is templated: [(1, "T")]
        templated_parameters: Vec<(usize, String)>,
        /// Concrete parameters: (target_index, concrete_type)
        /// e.g., for Abc<T, uint32_t> where uint32_t is concrete: [(1, "uint32_t")]
        /// e.g., for Abc<uint32_t, T> where uint32_t is concrete: [(0, "uint32_t")]
        concrete_parameters: Vec<(usize, String)>,
        /// Namespace path for this templated typedef
        namespace_path: Vec<String>,
        /// Pointer depth of the templated type
        depth: u64,
    },
}

impl<'a> Template {
    /// Creates a new struct template from its definition
    ///
    /// # Arguments
    /// * `def` - The template declaration line, like `template template_name`
    /// or simply `template_name`
    /// * `body` - The template body containing member definitions
    /// * `typenames` - List of template parameter names parsed from the definition line
    ///
    /// # Returns
    /// A new `Template::StructTemplate` instance
    pub fn new(def: &str, body: &str, typenames: Vec<String>, namespace_path: Vec<String>) -> Self {
        let name = parse_name(def).expect(&format!("Could not parse name from {def}"));
        Template::StructTemplate {
            name,
            typenames,
            body: body.to_string(),
            namespace_path,
        }
    }

    /// Creates a new typedef template from its definition
    ///
    /// # Arguments
    /// * `name` - The typedef name (e.g., "AA")
    /// * `typenames` - List of template parameter names (e.g., ["T"])
    /// * `target_template_name` - The base template name (e.g., "Abc")
    /// * `templated_parameters` - Templated parameters with positions
    /// * `concrete_parameters` - Concrete parameters with positions
    /// * `namespace_path` - Namespace path for this typedef
    ///
    /// # Returns
    /// A new `Template::TypedefTemplate` instance
    pub fn new_typedef(
        name: String,
        typenames: Vec<String>,
        target_template_name: String,
        templated_parameters: Vec<(usize, String)>,
        concrete_parameters: Vec<(usize, String)>,
        depth: u64,
        namespace_path: Vec<String>,
    ) -> Self {
        Template::TypedefTemplate {
            name,
            typenames,
            target_template_name,
            templated_parameters,
            concrete_parameters,
            depth,
            namespace_path,
        }
    }

    /// Gets the template name
    pub fn get_name(&self) -> &str {
        match self {
            Template::StructTemplate { name, .. } => name,
            Template::TypedefTemplate { name, .. } => name,
        }
    }

    /// Instantiates the template with concrete types in Binary Ninja
    ///
    /// # Arguments
    /// * `typenames` - Concrete type names to substitute for template parameters
    /// * `bv` - Binary Ninja binary view reference
    /// * `templates` - Reference to all templates for typedef resolution
    /// * `typedef_name` - Optional name for typedef templates (e.g., "AA" instead of "Abc")
    /// * `offset` - Pointer offset for the template definition. Default to 0 if unneeded.
    pub fn define<'b>(
        &self,
        typenames: Vec<String>,
        bv: &'a BinaryView,
        offset: i64,
        defined_namespace_path: &Vec<String>,
    ) -> Result<(), String> {
        match self {
            Template::StructTemplate {
                name,
                typenames: template_params,
                body,
                namespace_path,
            } => {
                // Typenames might be local to the defined namespace. Resolve the
                // full name (by checking until a valid type exists) and join the
                // instantiated name accordingly
                let mut instantiated_typenames = Vec::<String>::new();
                for t in typenames.iter() {
                    instantiated_typenames.push(resolve_type_name(&t, bv, defined_namespace_path));
                }
                // Instantiate type with full namespace resolution
                let instantiated_name = format!(
                    "{}<{}>",
                    self.get_full_name(),
                    instantiated_typenames.join(", ")
                );
                // Forward declare the type, which will be clobbered anyway.
                // Necessary for templated lists, arrays, trees, etc.
                let forward_decl = Type::structure(&StructureBuilder::new().finalize());
                log::info!("Forward declaring templated structure: {instantiated_name}");
                bv.define_user_type(&instantiated_name, &forward_decl);
                if typenames.len() != template_params.len() {
                    return Err(
                        "Provided typenames length does not match expected typenames length"
                            .to_string(),
                    );
                }
                let mut members = Vec::<Member>::new();
                for member in body.lines() {
                    if member.trim() == "" || member.trim().starts_with("//") {
                        continue;
                    }
                    log::debug!(
                        "Defining member {} for templated type {}",
                        member,
                        self.get_full_name()
                    );
                    // Create a new member and provide the typenames to swap in case
                    // the given member uses a typename
                    members.push(Member::new(
                        member,
                        bv,
                        Some(template_params),
                        Some(&typenames),
                        defined_namespace_path,
                    )?);
                }
                // Use self.name here, because new_from_members prepends the namespaces by default.
                let structure_name = format!("{}<{}>", name, instantiated_typenames.join(", "));
                Structure::new_from_members(
                    structure_name,
                    members,
                    offset,
                    namespace_path.clone(),
                )
                .define(bv)?;
                Ok(())
            }
            Template::TypedefTemplate {
                name,
                typenames: template_params,
                target_template_name,
                templated_parameters,
                concrete_parameters,
                depth,
                ..
            } => {
                if typenames.len() != template_params.len() {
                    return Err(
                        "Provided typenames length does not match expected typenames length"
                            .to_string(),
                    );
                }

                // Build the parameter list for the target template
                let max_index = templated_parameters
                    .iter()
                    .chain(concrete_parameters.iter())
                    .map(|(idx, _)| *idx)
                    .max()
                    .unwrap_or(0);

                let mut target_typenames = vec![String::new(); max_index + 1];

                // Fill in concrete parameters
                for (idx, concrete_type) in concrete_parameters {
                    target_typenames[*idx] = concrete_type.clone();
                }

                // Fill in templated parameters by substituting from our typenames
                for (idx, template_param) in templated_parameters {
                    if let Some(param_idx) =
                        template_params.iter().position(|p| p == template_param)
                    {
                        target_typenames[*idx] = typenames[param_idx].clone();
                    } else {
                        return Err(format!(
                            "Template parameter {} not found in typedef template {}",
                            template_param, name
                        ));
                    }
                }

                log::info!(
                    "Instantiating {} with parameters: {:?}",
                    target_template_name,
                    target_typenames
                );

                let full_typename =
                    format!("{target_template_name}<{}>", target_typenames.join(", "));
                let full_name = format!("{}<{}>", self.get_full_name(), typenames.join(", "));
                let mut typ = get_non_primitive_type_by_name(&full_typename, bv).ok_or(format!(
                    "Could not find type {} for templated typedef definition",
                    full_typename
                ))?;

                for _ in 0..*depth {
                    typ = Type::pointer(
                        &bv.default_arch().expect("Could not find core arch"),
                        typ.as_ref(),
                    );
                }

                bv.define_user_type(&full_name, &typ);

                Ok(())
            }
        }
    }

    /// Gets the full name including namespace prefix
    pub fn get_full_name(&self) -> String {
        match self {
            Template::StructTemplate {
                name,
                namespace_path,
                ..
            } => {
                if namespace_path.is_empty() {
                    name.clone()
                } else {
                    format!("{}::{}", namespace_path.join("::"), name)
                }
            }
            Template::TypedefTemplate {
                name,
                namespace_path,
                ..
            } => {
                if namespace_path.is_empty() {
                    name.clone()
                } else {
                    format!("{}::{}", namespace_path.join("::"), name)
                }
            }
        }
    }
}
