pub struct AppVars {
    pub name: String,
}

impl AppVars {
    pub fn new() -> Self {
        Self {
            name: "SafeMic".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_vars_name() {
        let vars = AppVars::new();
        assert_eq!(vars.name, "SafeMic");
    }
}
