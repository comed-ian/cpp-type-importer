use binaryninja::binary_view::BinaryView;
use binaryninja::command::{register_command, Command};
use binaryninja::logger::Logger;
use cpp_parser::parser::Parser;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::str::FromStr;

// TODO
// 1. Conflicting vtable function names (e.g., MyMethod)
// 2. Add comment lines to middle of structure and class
// 3. Add test for parsing template in function arguments
// 4. Bonus for within a templated type

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
        let mut parser = Parser::new(view.as_ref());
        for f in filenames {
            log::info!("Opening file {}", f.display());
            match File::open(&f) {
                Ok(mut file) => {
                    let mut contents = String::new();
                    match file.read_to_string(&mut contents) {
                        Ok(_) => {
                            log::info!("Successfully read {}, parsing C++ types...", f.display());
                            match parser.parse(&contents) {
                                Err(e) => log::error!(
                                    "Could not parse types from file {}: {e}",
                                    f.display()
                                ),
                                Ok(_) => {
                                    log::info!("C++ type import completed for file {}", f.display())
                                }
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
        .with_level(log::LevelFilter::Debug)
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
