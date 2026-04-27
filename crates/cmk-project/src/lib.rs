pub mod cmake;
pub mod cmake_ast;
pub mod default;

pub use cmake::{CMakeProject, Target, get_project_root};
pub use default::{Template, load_template};
