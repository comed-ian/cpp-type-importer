use binaryninja::binary_view::{BinaryView, BinaryViewExt};
use binaryninja::types::Type;

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
    array_size: Option<u64>,
    namespace_path: Vec<String>,
}

impl<'a> Typedef {
    pub fn new(def: &str, namespace_path: Vec<String>) -> Self {
        let (typ, name, depth, array_size) =
            parse_member_definition(def).expect("Could not parse typedef definition");
        Self {
            name,
            typ,
            depth,
            array_size,
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
        // TODO parse self.typ for templated names and resolve within current namespace.
        // For example, if current namespace is A and A::MyStruct exists, then resolve
        // B::template_name<MyStruct> --> B::template_name<A::MyStruct>
        let mut target_type = if let Some(tt) = is_primitive(&self.typ) {
            tt
        } else {
            Member::define_type(&self.typ, self.depth, bv, &self.namespace_path)?
        };

        // If this is an array typedef, wrap the type in an array
        if let Some(array_size) = self.array_size {
            target_type = Type::array(target_type.as_ref(), array_size);
        }

        log::info!("Got typedef target type: {}", target_type);

        // Construct full name with namespace prefix
        let full_name = self.get_full_name();
        bv.define_user_type(&full_name, &target_type);
        Ok(())
    }
}
