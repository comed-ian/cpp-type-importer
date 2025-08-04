use binaryninja::binary_view::{BinaryView, BinaryViewExt};
use binaryninja::rc::Ref;
use binaryninja::types::{FunctionParameter, MemberAccess, MemberScope, StructureBuilder, Type};
use regex::Regex;

use crate::utils::*;

/// Represents different types of C++ class/struct members
///
/// This enum covers basic data members, function pointers, and template members
/// with their respective type information and comments.
#[derive(Debug, Clone)]
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
    /// An array data member with element type, name, and size
    Array {
        /// The member name
        name: String,
        /// Binary Ninja type reference for the array element type
        element_type: Ref<Type>,
        /// Array size (number of elements)
        size: u64,
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
    pub fn name(&self) -> String {
        match self {
            Self::Basic { name, .. } => name.clone(),
            Self::Array { name, .. } => name.clone(),
            Self::Function { name, .. } => name.clone(),
        }
    }
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
                Member::Array {
                    name: name1,
                    element_type: element_type1,
                    size: size1,
                    ..
                },
                Member::Array {
                    name: name2,
                    element_type: element_type2,
                    size: size2,
                    ..
                },
            ) => {
                name1 == name2
                    && element_type1.to_string() == element_type2.to_string()
                    && size1 == size2
            }
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
    pub fn define_type(
        t: &str,
        depth: u8,
        bv: &BinaryView,
        current_namespace: &Vec<String>,
    ) -> Result<Ref<Type>, String> {
        Self::define_type_with_namespace(t, depth, bv, current_namespace)
    }

    pub fn define_type_with_namespace(
        t: &str,
        depth: u8,
        bv: &BinaryView,
        current_namespace: &Vec<String>,
    ) -> Result<Ref<Type>, String> {
        let mut ptr_level = String::new();
        for _ in 0..depth {
            ptr_level.push('*');
        }
        log::debug!(
            "Defining type: {}{} with current namespaces {:?}",
            t,
            ptr_level,
            current_namespace
        );
        let mut typ = if let Some(tt) = is_primitive(t) {
            tt
        } else {
            // Try to resolve the type with namespace resolution
            let resolved_type = resolve_type_name(t, bv, current_namespace);
            if let Some(tt) = get_non_primitive_type_by_name(&resolved_type, bv) {
                tt
            } else {
                return Err(format!("Could not find type: {}", resolved_type));
            }
        };
        for _ in 0..depth {
            typ = Type::pointer(
                &bv.default_arch().expect("Could not find core arch"),
                typ.as_ref(),
            );
        }
        Ok(typ)
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
    pub fn new(
        def: &str,
        bv: &BinaryView,
        template_members: Option<&Vec<String>>,
        template_defs: Option<&Vec<String>>,
        current_namespace: &Vec<String>,
    ) -> Result<Self, String> {
        // Remove any trailing comments from the current line. If comments are
        // meaningful (e.g., the `offset=` in a Class definition), these
        // should be parsed ahead of time.
        let def = def.split_once(';').map_or(def, |(before, _)| before).trim();
        if def == "" || def.starts_with("//") {
            return Err(format!("Member definition {def} is invalid"));
        };
        let (typ, name, depth, arrsize) =
            parse_member_definition(def).expect("Could not parse member definition");
        log::info!(
            "Got member definition type={}, name={}, depth={}, arrsize={:x?}",
            typ,
            name,
            depth,
            arrsize
        );
        if let Some(_) = is_primitive(&typ) {
            // Is primitive
            let typ = Self::define_type(&typ, depth, bv, current_namespace)?;
            match arrsize {
                Some(l) => {
                    return Ok(Member::Array {
                        name,
                        element_type: typ,
                        size: l,
                        comments: vec![],
                    });
                }
                None => {
                    return Ok(Member::Basic {
                        name,
                        typ,
                        comments: vec![],
                    });
                }
            }
        } else {
            // Is not primitive
            let mut def = def.to_string();
            // Try and replace templated member types, if they exist
            if let Some(t_members) = template_members {
                if template_defs.is_none() {
                    return Err(format!(
                        "Attempting to process templated member `{def}` but missing defs"
                    ));
                }
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
            // Supports arguments that are function pointers.
            let func_regex = Regex::new(r"^(.*?) \(\*(.*?)\)\((.*)\)$").unwrap();
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
                // TODO allow [] in arguments to function
                let (return_type, _, depth, _) = parse_member_definition(return_type)
                    .ok_or("Could not parse function member return type")?;
                let args = parse_function_arguments(args);
                log::info!("Parsed function args: {:?}", args);
                let mut defined_args = vec![];
                for a in args {
                    // Allow function pointers within arguments
                    if let Some(_) = func_regex.captures(&a) {
                        if let Member::Function { name, args, ret } =
                            Member::new(&a, bv, template_members, template_defs, current_namespace)?
                        {
                            let func_type = Self::new_function_type(&ret, &args, bv)?;
                            defined_args.push((name, func_type));
                        } else {
                            return Err(
                                "Incorrect member type returned when parsing a function argument"
                                    .to_string(),
                            );
                        }
                    } else {
                        let (typ, name, depth, _) = parse_member_definition(&a)
                            .ok_or("Could not parse argument to function definition")?;
                        defined_args.push((
                            name,
                            Self::define_type_with_namespace(&typ, depth, bv, current_namespace)?,
                        ));
                    }
                }

                return Ok(Member::Function {
                    name: name.to_string(),
                    ret: Self::define_type_with_namespace(
                        &return_type,
                        depth,
                        bv,
                        current_namespace,
                    )?,
                    args: defined_args,
                });
            } else {
                // Not a function definition
                let (typ, name, depth, arrsize) =
                    parse_member_definition(&def).expect(&format!("Could not parse {def}"));
                let typ = Self::define_type_with_namespace(&typ, depth, bv, current_namespace)?;
                match arrsize {
                    Some(l) => {
                        return Ok(Member::Array {
                            name,
                            element_type: typ,
                            size: l,
                            comments: vec![],
                        });
                    }
                    None => {
                        return Ok(Member::Basic {
                            name,
                            typ,
                            comments: vec![],
                        });
                    }
                }
            }
        }
    }
    pub fn new_function_type(
        ret: &Ref<Type>,
        args: &Vec<(String, Ref<Type>)>,
        bv: &BinaryView,
    ) -> Result<Ref<Type>, String> {
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
            &bv.default_arch().ok_or("Could not find default arch")?,
            func.as_ref(),
        );
        Ok(func)
    }
    pub fn define<'b>(
        &self,
        offset: Option<u64>,
        builder: &mut StructureBuilder,
        bv: &'a BinaryView,
    ) -> Result<(), String> {
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
            Member::Array {
                name,
                element_type,
                size,
                ..
            } => {
                log::debug!("Adding array: {}", name);
                let typ = Type::array(element_type, *size);
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
                let func = Self::new_function_type(ret, args, bv)?;
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
        Ok(())
    }
}
