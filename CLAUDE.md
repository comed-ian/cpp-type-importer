# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This is a Binary Ninja plugin written in Rust that imports C++ types from header files (.hpp) into Binary Ninja's type system. The plugin parses C++ structures, templates, classes with inheritance, and creates corresponding Binary Ninja types for reverse engineering analysis.

## Build System and Development Commands

### Build
```bash
# Build the Rust plugin
cargo build --release
```

### Development Environment
The project uses Nix for development environment management:
```bash
# Enter development shell with Helix editor and Rust toolchain
nix-shell
```

### Testing
```bash
# Build and test release version
cargo build --release
cargo test --release
```

### Rust Toolchain
- Uses Rust 1.87.0 (pinned in `rust-toolchain.toml`)
- Requires `rust-src` and `rust-analyzer` components
- Builds as a C dynamic library (`cdylib`) for Binary Ninja integration

## Architecture

### Core Components

#### Parser (`cpp_type_importer/src/lib.rs`)
- **`Parser`**: Main parsing engine that processes C++ header content with namespace support
- **`Structure`**: Represents C++ struct definitions with members, namespace support, and packed attribute support
- **`Template`**: Handles templated types with parameter substitution and instantiation support
- **`Enum`**: Represents C++ enum definitions with optional size and value assignments
- **`Member`**: Enum representing different member types (basic, function)
- **`Class`**: Represents C++ class definitions with inheritance, vtables, and member overrides
- **`Typedef`**: Represents type aliases with namespace support

#### Type System Integration
- **`is_primitive()`**: Maps C++ primitive types to Binary Ninja types
- **`Member::define_type()`**: Creates Binary Ninja types from C++ type strings with namespace resolution
- **`Structure::define()`**: Creates Binary Ninja struct types using `StructureBuilder`
- **`Enum::define()`**: Creates Binary Ninja enum types using `EnumerationBuilder`
- **`Class::define()`**: Creates Binary Ninja class types with inheritance and vtable support

#### Parsing Functions
- **`parse_template_definition()`**: Parses template parameter lists
- **`parse_template_instantiation()`**: Parses template instantiations with nested types
- **`parse_member_definition()`**: Parses member variable declarations
- **`parse_name()`**: Extracts names from class/struct definitions
- **`get_pointer_depth()`**: Determines pointer indirection levels
- **`parse_namespace()`**: Handles namespace definitions and nested namespaces
- **`parse_packed_attribute()`**: Parses `__attribute__((packed))` for structures

### Plugin Architecture
- **`ImportCppTypesCommand`**: Binary Ninja command that reads `test.hpp` and imports types
- **`CorePluginInit()`**: Plugin entry point that registers the command
- Uses Binary Ninja's command system for user interaction

### Supported C++ Features
- Basic structs with primitive and pointer members
- Function pointers as struct members
- Template definitions and instantiations (including nested templates)
- Forward declarations
- Multi-level pointer indirection (`T**`, `T***`, etc.)
- Enums with optional size specification and explicit value assignments
- **Namespace support** with nested namespaces and proper type resolution
- **Packed structures** with `__attribute__((packed))` support
- **Typedefs** with namespace support
- **Class inheritance** with vtable parsing and member overrides
- **Multiple inheritance** support with vtable handling

### Advanced Features
- **Vtable parsing**: Extracts virtual methods from class definitions
- **Method overrides**: Parses override comments and tracks inheritance chains
- **Namespace-aware type resolution**: Resolves types within namespace hierarchy
- **Template instantiation**: Creates concrete types from template definitions
- **Multiple inheritance vtables**: Handles complex inheritance scenarios

### Current Limitations
- Limited error handling for malformed C++ syntax
- Some template instantiation edge cases
- Unions are not supported
- Complex templated inheritance scenarios may need refinement

## Dependencies

### Binary Ninja API
- `binaryninja`: Main API bindings
- `binaryninjacore-sys`: Core system bindings
- Both use dev branch for latest features

### External Crates
- `regex`: For pattern matching in C++ parsing
- `log`: For debug logging

## Test Files
- `test.hpp`: Comprehensive C++ header file used for testing parsing functionality
- Contains examples of structs, templates, function pointers, enums, classes with inheritance
- **Namespace examples**: Nested namespaces (QQQ::RRR) with type definitions
- **Packed structures**: `__attribute__((packed))` examples
- **Multiple inheritance**: Classes inheriting from multiple base classes
- **Template instantiations**: Complex nested template examples
- `test.c`: Additional test file
- `test.bndb`: Binary Ninja database for testing

## Plugin Integration
- Plugin metadata defined in `plugin.json`
- Builds as a Binary Ninja helper plugin
- Supports macOS, Windows, and Linux platforms
- Minimum Binary Ninja version: 5.0
- **Command**: "Import C++ Types" - imports from `test.hpp`
- **Logging**: Uses Binary Ninja's Logger system for debug output

## Recent Development Focus
The codebase has evolved significantly from a basic struct parser to a comprehensive C++ type importer with advanced features:
- **Namespace support** for complex C++ codebases
- **Multiple inheritance** with vtable handling
- **Class inheritance** with virtual method parsing
- **Packed structure** support for low-level analysis
- **Template instantiation** improvements