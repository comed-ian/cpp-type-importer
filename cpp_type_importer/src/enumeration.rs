use binaryninja::binary_view::{BinaryView, BinaryViewExt};
use binaryninja::types::{EnumerationBuilder, Type};

use crate::utils::parse_decimal_or_hex;

/// Represents a C++ enum with values and size specification
///
/// This structure stores enum name, underlying type size, and value mappings
/// to support Binary Ninja enum type creation.
#[derive(Debug, Clone)]
pub struct Enum {
    /// The name of the enum type
    name: String,
    /// The size of the underlying type in bytes
    size: u8,
    /// List of enum values as (name, numeric_value) pairs
    values: Vec<(String, u64)>,
    /// Namespace path for this enum
    namespace_path: Vec<String>,
}

impl<'a> Enum {
    /// Creates a new enum from its definition
    ///
    /// # Arguments
    /// * `name` - The enum name
    /// * `size` - Size in bytes of the underlying type
    /// * `body` - The enum body containing value definitions
    ///
    /// # Returns
    /// A new `Enum` instance with parsed values
    pub fn new(
        name: &str,
        size: u8,
        body: &str,
        namespace_path: Vec<String>,
    ) -> Result<Self, String> {
        let mut values = Vec::new();
        // Enum values increment from the prior value if not specified,
        // with the default being 0
        let mut current_value = 0u64;

        for line in body.lines() {
            // Each line has the form `VALUE,` or `VALUE=X,`
            let line = line.trim();
            if line.is_empty() || line.starts_with("//") {
                continue;
            }

            let line = line
                .split_once(',')
                .map_or(line, |(before, _)| before)
                .trim();

            if let Some(eq_pos) = line.find('=') {
                // Parse explicit assignment like "VALUE=1"
                let name = line[..eq_pos].trim().to_string();
                let value_str = line[eq_pos + 1..].trim();

                // Parse the numeric value
                match parse_decimal_or_hex(value_str)? {
                    Some(val) => {
                        let val = val as u64;
                        current_value = val;
                        values.push((name, current_value));
                    }
                    None => {
                        return Err(format!(
                            "Could not parse enum value '{}', using {}",
                            value_str, current_value
                        ))
                    }
                }
            } else {
                // No explicit assignment, use current_value
                values.push((line.to_string(), current_value));
            }

            current_value += 1;
        }

        Ok(Self {
            name: name.to_string(),
            size,
            values,
            namespace_path,
        })
    }

    /// Defines the enum in Binary Ninja's type system
    ///
    /// # Arguments
    /// * `bv` - Binary Ninja binary view reference
    ///
    /// # Returns
    /// `true` if the enum was successfully defined
    /// Gets the full name including namespace prefix
    pub fn get_full_name(&self) -> String {
        if self.namespace_path.is_empty() {
            self.name.clone()
        } else {
            format!("{}::{}", self.namespace_path.join("::"), self.name)
        }
    }

    pub fn define(&self, bv: &BinaryView) -> Result<(), String> {
        // Create an enumeration builder
        let mut builder = EnumerationBuilder::new();

        // Add each enum value to the builder with its correct numeric value
        for (name, value) in &self.values {
            builder.insert(name, *value);
        }

        let enumeration = builder.finalize();

        // Create the enum type with the specified width, default to 4 bytes
        let width = std::num::NonZeroUsize::new(self.size as usize)
            .unwrap_or_else(|| std::num::NonZeroUsize::new(4).unwrap());
        let enum_type = Type::enumeration(&enumeration, width, false);

        // Construct full name with namespace prefix
        let full_name = self.get_full_name();
        bv.define_user_type(&full_name, &enum_type);
        Ok(())
    }
}
