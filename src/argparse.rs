use std::collections::HashMap;

/// Arguments are interpreted as a map with list of values. They are supposed
/// to be specified like --foo 1 --bar --baz 2 3
// Does it make sense to have HashMap::from(Vec<RedisString>)? This would allow
// to parse arguments in different ways depending on the target type - possibly
// changing the syntax in the future and being more flexible. TODO consider this.
pub fn parse_args(args: Vec<String>) -> HashMap<String, Vec<String>> {
    let mut p = HashMap::new();
    let mut last_key = "".to_owned();
    for a in args.into_iter() {
        let a = a.to_string();
        if a.starts_with("--") {
            last_key = a;
            p.entry(last_key.clone()).or_insert(vec![]);
        } else {
            p.entry(last_key.clone()).or_insert(vec![]).push(a);
        }
    }
    p
}

#[test]
fn test_parse_args() {
    let target = {
        let mut m = HashMap::new();
        m.insert("--foo".to_owned(), vec!["1".to_owned(), "11".to_owned()]);
        m.insert("--bar".to_owned(), vec![]);
        m.insert("--baz".to_owned(), vec!["2".to_owned()]);
        m
    };

    assert_eq!(
        parse_args(
            vec!["--foo", "1", "11", "--bar", "--baz", "2"]
                .into_iter()
                .map(|s| s.to_owned())
                .collect()
        ),
        target
    );

    assert_eq!(
        parse_args(
            vec!["--foo", "1", "--foo", "11", "--baz", "2", "--bar"]
                .into_iter()
                .map(|s| s.to_owned())
                .collect()
        ),
        target
    );

    assert_eq!(parse_args(vec![]), HashMap::new());
}
