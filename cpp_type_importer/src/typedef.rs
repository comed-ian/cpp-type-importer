use binaryninja::binary_view::{BinaryView, BinaryViewExt};

use crate::Member;
use crate::{is_primitive, parse_member_definition};

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
        // TODO add [] to typedefs
        let (typ, name, depth, _) =
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

    pub fn define(&self, bv: &'a BinaryView) -> Result<(), String> {
        let target_type = if let Some(tt) = is_primitive(&self.typ) {
            tt
        } else {
            Member::define_type(&self.typ, self.depth, bv)?
        };
        log::info!("Got typedef target type: {}", target_type);

        // Construct full name with namespace prefix
        let full_name = self.get_full_name();
        bv.define_user_type(&full_name, &target_type);
        Ok(())
    }
}
