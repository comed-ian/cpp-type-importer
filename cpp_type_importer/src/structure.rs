use crate::{parse_name, parse_ptr_offset, Member};
use binaryninja::binary_view::{BinaryView, BinaryViewExt};
use binaryninja::types::{StructureBuilder, Type};

/// Represents a C++ struct with members and Binary Ninja type information
///
///
/// This structure stores the struct name, member definitions, and offset information
/// to support Binary Ninja struct type creation.
#[derive(Debug)]
pub struct Structure {
    /// The name of the struct
    pub name: String,
    /// List of struct members
    pub members: Vec<Member>,
    /// Base offset for the struct
    pub offset: i64,
    /// Whether the struct is packed (no padding)
    pub packed: bool,
    /// Namespace path for this structure
    pub namespace_path: Vec<String>,
}

impl<'a> Structure {
    /// Creates a new structure from its definition
    ///
    /// # Arguments
    /// * `def` - The struct declaration line, like `struct struct_name`
    /// or simply `struct_name`
    /// * `body` - The struct body containing member definitions
    /// * `bv` - Binary Ninja binary view reference
    ///
    /// # Returns
    /// A new `Structure` instance
    ///
    /// # Notes
    /// Note that this method clobbers any existing struct with the same
    /// name, as it forward-declares itself so that self-referrential members
    /// (e.g., linked list pointers) have a type to reference. Calling this
    /// method assumes that returned `Structure` will be subsequently defined
    /// with a call to [`Structure::define`].
    pub fn new<'b>(
        def: &str,
        body: &str,
        bv: &'a BinaryView,
        namespace_path: Vec<String>,
    ) -> Result<Self, String> {
        let name = parse_name(def).expect(&format!("Could not parse definition {def} for name"));
        // Forward declare the type, which will be clobbered anyway.
        // Necessary for templated lists, arrays, trees, etc.
        let forward_decl = Type::structure(&StructureBuilder::new().finalize());
        let full_name = if namespace_path.is_empty() {
            name.clone()
        } else {
            format!("{}::{}", namespace_path.join("::"), name)
        };
        log::info!("Forward declaring structure: {full_name}");
        bv.define_user_type(&full_name, &forward_decl);

        // Check for packed attribute in the definition
        let packed = def.contains("__attribute__((packed))");

        let mut offset = 0;

        let mut members = vec![];
        for (i, member) in body.lines().enumerate() {
            if i == 0 {
                // Check for __ptr_offset(X) directive from the first line of the structure's body
                offset = parse_ptr_offset(member)?.unwrap_or(0);
            }

            if member.trim().starts_with("//") {
                continue;
            }
            // Create a new member with no templated fields
            members.push(Member::new(member, bv, None, None, &namespace_path)?);
        }

        Ok(Self {
            name,
            members,
            offset,
            packed,
            namespace_path,
        })
    }

    /// Creates a new structure from pre-parsed members. Useful for coercing
    /// other types into a `Structure` for easy definition, such as [`Class`]
    /// and [`Template`].
    ///
    /// # Arguments
    /// * `name` - The pre-parsed struct name
    /// * `members` - Pre-parsed member list
    /// * `offset` - Base offset for the struct
    ///
    /// # Returns
    /// A new `Structure` instance
    pub fn new_from_members(
        name: String,
        members: Vec<Member>,
        offset: i64,
        namespace_path: Vec<String>,
    ) -> Self {
        Self {
            name,
            members,
            offset,
            packed: false,
            namespace_path,
        }
    }

    /// Defines the structure in Binary Ninja's type system
    ///
    /// # Arguments
    /// * `bv` - Binary Ninja binary view reference
    ///
    /// # Returns
    /// `true` if the structure was successfully defined
    pub fn define<'b>(&mut self, bv: &'a BinaryView) -> Result<(), String> {
        let full_name = self.get_full_name();
        log::debug!(
            "Defining {} structure {}",
            if self.packed { "(packed)" } else { "" },
            full_name
        );
        let mut builder = StructureBuilder::new();

        // Set packed flag if the structure is packed
        if self.packed {
            builder.packed(true);
        }

        // Set pointer offset if specified
        if self.offset != 0 {
            builder.pointer_offset(self.offset);
        }

        for m in self.members.iter_mut() {
            m.define(None, &mut builder, bv)?;
        }
        let s = Type::structure(&builder.finalize());
        bv.define_user_type(&full_name, &s);
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
