use crate::search::paths::is_markdown;
use std::path::Path;

pub fn existing_files_impl(paths: Vec<String>) -> Vec<String> {
    paths
        .into_iter()
        .filter(|value| {
            let path = Path::new(value);
            path.is_file() && is_markdown(path)
        })
        .collect()
}
