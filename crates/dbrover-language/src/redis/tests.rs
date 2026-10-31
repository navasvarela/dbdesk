use super::*;

fn commands(script: &str) -> Vec<&str> {
    split(script)
        .into_iter()
        .map(|range| &script[range])
        .collect()
}

fn bytes(arguments: &[&str]) -> Vec<Vec<u8>> {
    arguments
        .iter()
        .map(|argument| argument.as_bytes().to_vec())
        .collect()
}

#[test]
fn splits_one_command_per_line() {
    let cases: [(&str, &[&str]); 6] = [
        ("GET a\nSET b 1", &["GET a", "SET b 1"]),
        ("  GET a  \n\n\t\n  DEL b ", &["GET a", "DEL b"]),
        ("# only a comment\nPING", &["PING"]),
        ("  # an indented comment\nPING", &["PING"]),
        ("SET a \"one\ntwo\"", &["SET a \"one", "two\""]),
        ("", &[]),
    ];
    for (script, expected) in cases {
        assert_eq!(commands(script), expected, "{script:?}");
    }
}

#[test]
fn a_hash_sign_after_the_start_of_a_line_is_part_of_an_argument() {
    assert_eq!(commands("SET tag #red"), ["SET tag #red"]);
    assert_eq!(
        arguments("SET tag #red"),
        Ok(bytes(&["SET", "tag", "#red"]))
    );
}

#[test]
fn tokens_cover_the_text_and_tell_words_strings_numbers_and_comments_apart() {
    let text = "# note\nSET k \"a \\\" b\" 'c' -42 x\"y\"\n";
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
            (TokenKind::LineComment, "# note"),
            (TokenKind::Word, "SET"),
            (TokenKind::Word, "k"),
            (TokenKind::String, "\"a \\\" b\""),
            (TokenKind::String, "'c'"),
            (TokenKind::Number, "-42"),
            (TokenKind::Word, "x"),
            (TokenKind::String, "\"y\""),
        ]
    );
}

/// The same arguments `redis-cli` sends, quotes and escapes as `sdssplitargs` reads them.
#[test]
fn reads_arguments_as_redis_cli_does() {
    let cases: [(&str, Vec<Vec<u8>>); 7] = [
        ("  SET  key   value ", bytes(&["SET", "key", "value"])),
        (
            r#"SET "a key" "line\nbreak \"quoted\"""#,
            bytes(&["SET", "a key", "line\nbreak \"quoted\""]),
        ),
        (
            r#"SET k "\x41\x7a\xff\x0""#,
            vec![b"SET".to_vec(), b"k".to_vec(), b"Az\xffx0".to_vec()],
        ),
        (r"SET 'it\'s' 'a\nb'", bytes(&["SET", "it's", "a\\nb"])),
        (r#"SET pre"fix and" x"#, bytes(&["SET", "prefix and", "x"])),
        (r#"ECHO """#, bytes(&["ECHO", ""])),
        ("", Vec::new()),
    ];
    for (command, expected) in cases {
        assert_eq!(arguments(command), Ok(expected), "{command}");
    }
}

#[test]
fn refuses_quotes_redis_cli_refuses() {
    assert_eq!(arguments(r#"GET "open"#), Err(ArgumentsError::Unclosed));
    assert_eq!(arguments(r"GET 'open"), Err(ArgumentsError::Unclosed));
    assert_eq!(
        arguments(r#"GET "closed"after"#),
        Err(ArgumentsError::TextAfterQuote)
    );
}

#[test]
fn quoted_arguments_read_back_as_they_were() {
    let cases: [&[u8]; 8] = [
        b"plain",
        b"user:42",
        b"two words",
        b"",
        b"say \"hi\"\n",
        b"back\\slash and 'single'",
        b"\x00\xff\x7f binary",
        b"#not-a-comment",
    ];
    for argument in cases {
        let command = format!("SET {}", quote(argument));
        assert_eq!(
            arguments(&command),
            Ok(vec![b"SET".to_vec(), argument.to_vec()]),
            "{command}"
        );
    }
    assert_eq!(quote(b"user:42"), "user:42");
    assert_eq!(quote(b"two words"), "\"two words\"");
}

#[test]
fn finds_a_subcommand_before_its_container() {
    let (name, spec) = command(&bytes(&["config", "GET", "maxmemory"])).unwrap();
    assert_eq!(name.as_str(), "config|get");
    assert_eq!(name.to_string(), "CONFIG GET");
    assert_eq!(spec.name, "config|get");
    let (name, _) = command(&bytes(&["GET", "config"])).unwrap();
    assert_eq!(name.as_str(), "get");
    assert_eq!(command(&bytes(&["NOSUCH"])), None);
}

#[test]
fn arities_accept_what_the_server_does() {
    let get = commands::find("get").unwrap();
    assert!(get.accepts(2));
    assert!(!get.accepts(1));
    assert!(!get.accepts(3));
    let hset = commands::find("hset").unwrap();
    assert!(hset.accepts(4));
    assert!(hset.accepts(6));
    assert!(!hset.accepts(3));
}

#[test]
fn the_table_is_sorted_and_names_are_lowercase() {
    assert!(
        commands::COMMANDS
            .windows(2)
            .all(|pair| pair[0].name < pair[1].name)
    );
    assert!(commands::NAMES.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(
        commands::COMMANDS
            .iter()
            .all(|spec| spec.name == spec.name.to_ascii_lowercase())
    );
    assert!(
        commands::NAMES
            .iter()
            .all(|name| commands::find(name).is_some() && !name.contains('|'))
    );
}

#[test]
fn completes_command_names_with_their_syntax() {
    assert!(completion_words().contains(&"hset"));
    assert!(!completion_words().iter().any(|word| word.contains('|')));
    assert_eq!(syntax("HSET"), Some("key field value [field value ...]"));
    assert_eq!(syntax("nosuch"), None);
}

#[test]
fn warns_of_the_commands_that_touch_every_key() {
    assert!(hazard("FLUSHALL").is_some());
    assert!(hazard("  flushdb async").is_some());
    assert!(hazard("KEYS user:*").is_some());
    assert_eq!(hazard("GET keys"), None);
    assert_eq!(hazard("SCAN 0 MATCH *"), None);
}

#[test]
fn refuses_to_guess_what_text_without_exactly_one_command_does() {
    assert_eq!(classify("  # nothing\n"), Err(Unclassified::Empty));
    assert_eq!(classify("GET a\nGET b"), Err(Unclassified::Several(2)));
}

/// Each command in `classify.txt` follows a comment naming what it must be classified as,
/// `# expect: read` (or `write`, `transaction`, `admin`, `unparseable`), as the test language's
/// corpus does.
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
    assert!(checked > 60, "only {checked} commands checked");
}

/// Every command the table lists is classified one way or another, and the plan's exceptions
/// win over the flags (`docs/nosql.md`, N1).
#[test]
fn every_listed_command_has_a_kind() {
    for spec in commands::COMMANDS {
        let command = spec.name.replace('|', " ");
        assert!(classify(&command).is_ok(), "{command}");
    }
    assert_eq!(classify("FLUSHALL"), Ok(StatementKind::Admin));
    assert_eq!(classify("EVAL x 0"), Ok(StatementKind::Write));
}
