use super::*;

fn commands(script: &str) -> Vec<&str> {
    split(script)
        .into_iter()
        .map(|range| &script[range])
        .collect()
}

#[test]
fn splits_one_command_per_line() {
    let cases: [(&str, &[&str]); 5] = [
        ("get a\nset b 1", &["get a", "set b 1"]),
        ("  get a  \n\n\t\n  del b ", &["get a", "del b"]),
        ("# only a comment\nping # and one after", &["ping"]),
        ("set a \"one\ntwo\"", &["set a \"one", "two\""]),
        ("", &[]),
    ];
    for (script, expected) in cases {
        assert_eq!(commands(script), expected, "{script:?}");
    }
}

#[test]
fn tokens_cover_the_text_and_tell_words_strings_numbers_and_comments_apart() {
    let text = "set k \"a \\\" b\" 42 # note\n";
    let tokens = tokenize(text);
    let covered: String = tokens
        .iter()
        .map(|token| &text[token.range.clone()])
        .collect();
    assert_eq!(covered, text);
    let kinds: Vec<(TokenKind, &str)> = tokens
        .iter()
        .filter(|token| token.kind != TokenKind::Whitespace)
        .map(|token| (token.kind, &text[token.range.clone()]))
        .collect();
    assert_eq!(
        kinds,
        [
            (TokenKind::Word, "set"),
            (TokenKind::Word, "k"),
            (TokenKind::String, "\"a \\\" b\""),
            (TokenKind::Number, "42"),
            (TokenKind::LineComment, "# note"),
        ]
    );
}

#[test]
fn keywords_are_the_verbs_sorted_and_ignore_case() {
    assert!(KEYWORDS.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(KEYWORDS.len(), VERBS.len());
    assert!(is_keyword("GET"));
    assert!(!is_keyword("user:1"));
}

#[test]
fn refuses_to_guess_what_text_without_exactly_one_command_does() {
    assert_eq!(classify("  # nothing\n"), Err(Unclassified::Empty));
    assert_eq!(classify("get a\nget b"), Err(Unclassified::Several(2)));
}

/// Each command in `classify.txt` follows a comment naming what it must be classified as,
/// `# expect: read` (or `write`, `schema`, `transaction`, `admin`, `unparseable`), as each SQL
/// dialect's corpus in `dbrover-sql/tests/classify/` does.
#[test]
fn classifies_its_corpus() {
    const EXPECT: &str = "# expect:";
    let corpus = include_str!("classify.txt");
    let mut previous_end = 0;
    let mut checked = 0;
    for range in split(corpus) {
        let before = &corpus[previous_end..range.start];
        previous_end = range.end;
        let command = &corpus[range];
        let expected = before
            .lines()
            .filter_map(|line| line.trim().strip_prefix(EXPECT))
            .next_back()
            .unwrap_or_else(|| panic!("no `{EXPECT}` line before {command}"))
            .trim();
        let actual = match classify(command) {
            Ok(StatementKind::Read) => "read",
            Ok(StatementKind::Write) => "write",
            Ok(StatementKind::Schema) => "schema",
            Ok(StatementKind::Transaction) => "transaction",
            Ok(StatementKind::Admin) => "admin",
            Err(Unclassified::Unparseable(_)) => "unparseable",
            Err(error) => panic!("{command}: {error}"),
        };
        assert_eq!(actual, expected, "{command}");
        checked += 1;
    }
    assert!(checked > 20, "only {checked} commands checked");
}
