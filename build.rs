use serde::Deserialize;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::PathBuf;

#[derive(Deserialize)]
struct LanguagesConfig {
    languages: Vec<Language>,
}

#[derive(Deserialize)]
struct FilesConfig {
    #[serde(default)]
    filenames: Vec<FilenameRule>,
    #[serde(default)]
    filename_prefixes: Vec<FilenamePrefixRule>,
    #[serde(default)]
    filename_suffixes: Vec<FilenameSuffixRule>,
}

#[derive(Deserialize)]
struct FilenameRule {
    name: String,
    language: String,
}

#[derive(Deserialize)]
struct FilenamePrefixRule {
    prefix: String,
    language: String,
}

#[derive(Deserialize)]
struct FilenameSuffixRule {
    suffix: String,
    language: String,
}

#[derive(Deserialize)]
struct Language {
    name: String,
    extensions: Vec<String>,
    line_comments: Vec<String>,
    block_comments: Vec<[String; 2]>,
    #[serde(default)]
    disambiguation: Vec<Disambiguation>,
}

#[derive(Deserialize)]
struct Disambiguation {
    regex: String,
    score: u32,
}

fn main() {
    println!("cargo:rerun-if-changed=data/languages");
    println!("cargo:rerun-if-changed=data/files.toml");

    let languages = load_languages();
    let files: FilesConfig = toml::from_str(
        &fs::read_to_string("data/files.toml").expect("failed to read data/files.toml"),
    )
    .expect("failed to parse data/files.toml");

    let language_ids = languages
        .languages
        .iter()
        .enumerate()
        .map(|(index, language)| (language.name.as_str(), index))
        .collect::<BTreeMap<_, _>>();

    let mut extensions = BTreeMap::<String, Vec<usize>>::new();
    for (language_id, language) in languages.languages.iter().enumerate() {
        for extension in &language.extensions {
            extensions
                .entry(extension.to_ascii_lowercase())
                .or_default()
                .push(language_id);
        }
    }

    let mut generated = String::new();
    generated.push_str("pub fn language_named(name: &str) -> Option<LanguageId> {\n");
    generated.push_str("    match name {\n");
    for (name, language_id) in &language_ids {
        generated.push_str(&format!(
            "        {:?} => Some(LanguageId({})),\n",
            name, language_id
        ));
    }
    generated.push_str("        _ => None,\n");
    generated.push_str("    }\n");
    generated.push_str("}\n\n");
    generated.push_str("#[allow(dead_code)]\npub const LANGUAGES: &[LanguageDef] = &[\n");
    for language in &languages.languages {
        generated.push_str("    LanguageDef {\n");
        generated.push_str(&format!("        name: {:?},\n", language.name));
        generated.push_str(&format!(
            "        line_comments: &{:?},\n",
            language.line_comments
        ));
        generated.push_str("        block_comments: &[");
        for [start, end] in &language.block_comments {
            generated.push_str(&format!("({start:?}, {end:?}),"));
        }
        generated.push_str("],\n");
        generated.push_str("    },\n");
    }
    generated.push_str("];\n\n");

    generated.push_str("pub const DISAMBIGUATION_RULES: &[&[DisambiguationRule]] = &[\n");
    for language in &languages.languages {
        generated.push_str("    &[");
        for rule in &language.disambiguation {
            let regex = format!("(?-u:{})", rule.regex);
            generated.push_str(&format!(
                "DisambiguationRule {{ regex: {:?}, score: {} }},",
                regex, rule.score
            ));
        }
        generated.push_str("],\n");
    }
    generated.push_str("];\n\n");

    generated.push_str("pub fn filename_language(filename: &str) -> Option<LanguageId> {\n");
    generated.push_str("    match filename {\n");
    for rule in &files.filenames {
        let language_id = language_ids
            .get(rule.language.as_str())
            .unwrap_or_else(|| panic!("unknown language {:?} in filename rule", rule.language));
        generated.push_str(&format!(
            "        {:?} => Some(LanguageId({})),\n",
            rule.name, language_id
        ));
    }
    generated.push_str("        _ => {\n");
    for rule in &files.filename_prefixes {
        let language_id = language_ids.get(rule.language.as_str()).unwrap_or_else(|| {
            panic!(
                "unknown language {:?} in filename prefix rule",
                rule.language
            )
        });
        generated.push_str(&format!(
            "            if filename.starts_with({:?}) {{ return Some(LanguageId({})); }}\n",
            rule.prefix, language_id
        ));
    }
    for rule in &files.filename_suffixes {
        let language_id = language_ids.get(rule.language.as_str()).unwrap_or_else(|| {
            panic!(
                "unknown language {:?} in filename suffix rule",
                rule.language
            )
        });
        generated.push_str(&format!(
            "            if filename.ends_with({:?}) {{ return Some(LanguageId({})); }}\n",
            rule.suffix, language_id
        ));
    }
    generated.push_str("            None\n");
    generated.push_str("        }\n");
    generated.push_str("    }\n");
    generated.push_str("}\n\n");

    generated.push_str("pub const EXTENSION_LANGUAGES: &[(&str, &[LanguageId])] = &[\n");
    for (extension, language_ids) in extensions {
        generated.push_str(&format!("    ({extension:?}, &["));
        for language_id in language_ids {
            generated.push_str(&format!("LanguageId({language_id}),"));
        }
        generated.push_str("]),\n");
    }
    generated.push_str("];\n\n");

    generated.push_str("pub fn extension_languages(extension: &str) -> &'static [LanguageId] {\n");
    generated.push_str(
        "    match EXTENSION_LANGUAGES.binary_search_by(|(candidate, _)| cmp_ignore_ascii_case(candidate, extension)) {\n",
    );
    generated.push_str("        Ok(index) => EXTENSION_LANGUAGES[index].1,\n");
    generated.push_str("        Err(_) => &[],\n");
    generated.push_str("    }\n");
    generated.push_str("}\n");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR not set"));
    fs::write(out_dir.join("languages.rs"), generated)
        .expect("failed to write generated languages.rs");
}

fn load_languages() -> LanguagesConfig {
    let mut paths = fs::read_dir("data/languages")
        .expect("failed to read data/languages")
        .map(|entry| entry.expect("failed to read language data entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "toml")
        })
        .collect::<Vec<_>>();
    paths.sort();

    let mut languages = Vec::new();
    for path in paths {
        println!("cargo:rerun-if-changed={}", path.display());
        let config: LanguagesConfig = toml::from_str(
            &fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display())),
        )
        .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()));
        languages.extend(config.languages);
    }

    LanguagesConfig { languages }
}
