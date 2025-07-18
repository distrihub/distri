use std::collections::HashMap;

pub fn replace_variables(prompt: &str, variables: &HashMap<String, String>) -> String {
    let mut prompt = prompt.to_owned();
    for (key, value) in variables {
        prompt = prompt.replace(&format!("{{{}}}", key), value);
    }
    prompt
}
