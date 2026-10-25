//! Which servers exist (a built-in table plus the user's `lsp.json`), how
//! their programs are found, and the folder each one works in.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A language server: how to run it and for which languages.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ServerConfig {
    pub name: String,
    /// The program (a name looked for on `PATH`, or an absolute path) and
    /// its arguments.
    pub command: Vec<String>,
    /// ochrepad's language names (as the status bar shows them).
    pub languages: Vec<String>,
    /// Answers to the server's `workspace/configuration` requests, by
    /// section (`{"rust-analyzer": {...}}`).
    pub settings: Value,
    pub initialization_options: Value,
}

/// `lsp.json` in the config folder.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct LspFile {
    /// Added servers; one named like a built-in one replaces it.
    pub servers: Vec<ServerConfig>,
    /// Servers never started.
    pub disabled: Vec<String>,
}

impl LspFile {
    pub fn parse(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| e.to_string())
    }
}

fn server(name: &str, command: &[&str], languages: &[&str]) -> ServerConfig {
    ServerConfig {
        name: name.into(),
        command: command.iter().map(|s| (*s).into()).collect(),
        languages: languages.iter().map(|s| (*s).into()).collect(),
        settings: Value::Null,
        initialization_options: Value::Null,
    }
}

/// The usual servers, in the order they are looked for (the first one
/// found serves its languages).
pub fn builtin() -> Vec<ServerConfig> {
    const PYTHON: &[&str] = &["Python"];
    vec![
        server("rust-analyzer", &["rust-analyzer"], &["Rust"]),
        server(
            "basedpyright",
            &["basedpyright-langserver", "--stdio"],
            PYTHON,
        ),
        server("pyright", &["pyright-langserver", "--stdio"], PYTHON),
        server("pylsp", &["pylsp"], PYTHON),
        server("jedi-language-server", &["jedi-language-server"], PYTHON),
        server("ty", &["ty", "server"], PYTHON),
        server("gopls", &["gopls"], &["Go"]),
        server(
            "clangd",
            &["clangd"],
            &["C", "C++", "Objective-C", "Objective-C++"],
        ),
        server(
            "typescript-language-server",
            &["typescript-language-server", "--stdio"],
            &["JavaScript", "TypeScript", "TypeScriptReact"],
        ),
        server("marksman", &["marksman", "server"], &["Markdown"]),
    ]
}

/// The servers to look for: the user's first (replacing built-in ones of
/// the same name), then the built-in ones, without the disabled ones.
pub fn servers(user: &LspFile) -> Vec<ServerConfig> {
    let mut out: Vec<ServerConfig> = user
        .servers
        .iter()
        .filter(|s| !s.name.is_empty() && !s.command.is_empty())
        .cloned()
        .collect();
    for b in builtin() {
        if !out.iter().any(|s| s.name == b.name) {
            out.push(b);
        }
    }
    out.retain(|s| !user.disabled.contains(&s.name));
    out
}

/// The protocol's language id for one of ochrepad's language names.
pub fn language_id(language: &str) -> String {
    match language {
        "C++" => "cpp".into(),
        "Objective-C" => "objective-c".into(),
        "Objective-C++" => "objective-cpp".into(),
        "TypeScriptReact" => "typescriptreact".into(),
        "Bourne Again Shell (bash)" => "shellscript".into(),
        other => other.to_lowercase().replace(' ', ""),
    }
}

/// The program's absolute path. A path in the config must be absolute; a
/// name is looked for in the `PATH` folders that are absolute and outside
/// every folder in `avoid` (the project), so a program planted in a
/// repository is never run.
pub fn resolve_program(program: &str, avoid: &[&Path]) -> Option<PathBuf> {
    resolve_in(
        program,
        &std::env::var_os("PATH").unwrap_or_default(),
        avoid,
    )
}

/// [`resolve_program`] with `path` for the `PATH` variable.
fn resolve_in(program: &str, path: &std::ffi::OsStr, avoid: &[&Path]) -> Option<PathBuf> {
    let given = Path::new(program);
    if given.components().count() > 1 || given.is_absolute() {
        return (given.is_absolute() && given.is_file()).then(|| given.to_path_buf());
    }
    let extensions: Vec<String> = if cfg!(windows) {
        let ext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        ext.split(';')
            .filter(|e| e.starts_with('.'))
            .map(str::to_lowercase)
            .collect()
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(path) {
        if !dir.is_absolute() || avoid.iter().any(|a| dir.starts_with(a)) {
            continue;
        }
        for ext in &extensions {
            let candidate = dir.join(format!("{program}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Files that mark a project's top folder.
pub const ROOT_MARKERS: &[&str] = &[
    "Cargo.toml",
    "pyproject.toml",
    "setup.py",
    "package.json",
    "go.mod",
    "CMakeLists.txt",
    "compile_commands.json",
    "dbt_project.yml",
    ".git",
];

/// The folder a server works in for `file`: the open folder when the file
/// is in it, else the nearest folder above the file with a marker, else
/// the file's folder. (Reads the disk.)
pub fn find_root(file: &Path, open_folder: Option<&Path>) -> Option<PathBuf> {
    if let Some(open) = open_folder
        && file.starts_with(open)
    {
        return Some(open.to_path_buf());
    }
    let dir = file.parent()?;
    for d in dir.ancestors() {
        if ROOT_MARKERS.iter().any(|m| d.join(m).exists()) {
            return Some(d.to_path_buf());
        }
    }
    Some(dir.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_users_servers_come_first_and_replace_built_in_ones() {
        let user = LspFile::parse(
            r#"{
                "servers": [
                    {"name": "dbt", "command": ["dbt-ls", "--stdio"], "languages": ["SQL"]},
                    {"name": "gopls", "command": ["/opt/gopls"], "languages": ["Go"],
                     "settings": {"gopls": {"staticcheck": true}}}
                ],
                "disabled": ["pylsp"]
            }"#,
        )
        .unwrap();
        let all = servers(&user);
        assert_eq!(all[0].name, "dbt");
        assert_eq!(all[1].command, ["/opt/gopls"]);
        assert_eq!(all.iter().filter(|s| s.name == "gopls").count(), 1);
        assert!(!all.iter().any(|s| s.name == "pylsp"));
        assert!(all.iter().any(|s| s.name == "rust-analyzer"));
        assert!(LspFile::parse("{\"servers\": 3}").is_err());
    }

    #[test]
    fn language_ids() {
        assert_eq!(language_id("Rust"), "rust");
        assert_eq!(language_id("C++"), "cpp");
        assert_eq!(language_id("TypeScriptReact"), "typescriptreact");
    }

    #[test]
    fn programs_in_the_project_are_not_run() {
        let dir = tempfile::tempdir().unwrap();
        let name = "ochrepad-test-server";
        let file = dir.path().join(if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_owned()
        });
        std::fs::write(&file, "").unwrap();
        // Relative paths in the config are refused; absolute ones are used.
        assert_eq!(resolve_program("sub/x", &[]), None);
        assert_eq!(
            resolve_program(&file.to_string_lossy(), &[]),
            Some(file.clone())
        );
        // On PATH: found, unless it is in the project or PATH names it
        // relatively.
        let path = std::env::join_paths([Path::new("."), dir.path()]).unwrap();
        assert_eq!(resolve_in(name, &path, &[]), Some(file));
        assert_eq!(resolve_in(name, &path, &[dir.path()]), None);
    }

    #[test]
    fn the_root_is_the_open_folder_or_a_marked_one() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("proj");
        std::fs::create_dir_all(proj.join("src")).unwrap();
        std::fs::write(proj.join("Cargo.toml"), "").unwrap();
        let file = proj.join("src").join("main.rs");
        assert_eq!(find_root(&file, None), Some(proj.clone()));
        assert_eq!(
            find_root(&file, Some(dir.path())),
            Some(dir.path().to_path_buf())
        );
        let loose = dir.path().join("loose.rs");
        let root = find_root(&loose, None).unwrap();
        assert!(loose.starts_with(&root));
    }
}
