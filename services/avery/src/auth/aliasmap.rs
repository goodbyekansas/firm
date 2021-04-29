use std::collections::HashMap;

#[derive(Debug)]
pub struct AliasMap {
    inner: HashMap<String, Vec<String>>,
}

impl AliasMap {
    pub fn get(&self, key: &str) -> Option<&[String]> {
        self.inner.get(key).map(Vec::as_slice)
    }
}

impl From<HashMap<String, Vec<String>>> for AliasMap {
    fn from(map: HashMap<String, Vec<String>>) -> Self {
        Self {
            inner: map.values().fold(HashMap::new(), |mut m, vec| {
                vec.iter().for_each(|value| {
                    m.insert(value.to_owned(), vec.clone());
                });
                m
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn from_multi_map() {
        let mut map = HashMap::new();
        map.insert(
            "a".to_owned(),
            vec!["x".to_owned(), "y".to_owned(), "z".to_owned()],
        );
        map.insert("b".to_owned(), vec!["1".to_owned(), "2".to_owned()]);
        let alias_map: AliasMap = map.into();
        assert_eq!(
            alias_map.get("x"),
            Some(vec!["x".to_owned(), "y".to_owned(), "z".to_owned()].as_slice())
        );
        assert_eq!(
            alias_map.get("y"),
            Some(vec!["x".to_owned(), "y".to_owned(), "z".to_owned()].as_slice())
        );
        assert_eq!(
            alias_map.get("z"),
            Some(vec!["x".to_owned(), "y".to_owned(), "z".to_owned()].as_slice())
        );
        assert_eq!(
            alias_map.get("1"),
            Some(vec!["1".to_owned(), "2".to_owned()].as_slice())
        );
        assert_eq!(
            alias_map.get("2"),
            Some(vec!["1".to_owned(), "2".to_owned()].as_slice())
        );
    }
}
