use super::*;

fn commands(script: &str) -> Vec<&str> {
    split(script)
        .into_iter()
        .map(|range| &script[range])
        .collect()
}

fn string(text: &str) -> Value {
    Value::String(text.to_owned())
}

fn number(text: &str) -> Value {
    Value::Number(text.to_owned())
}

fn object(fields: &[(&str, Value)]) -> Value {
    Value::Object(
        fields
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
    )
}

fn call(text: &str) -> Call {
    match parse(text) {
        Ok(Command::Call(call)) => call,
        other => panic!("{text}: {other:?}"),
    }
}

fn error(text: &str) -> ParseError {
    match parse(text) {
        Err(error) => error,
        Ok(command) => panic!("{text} parsed as {command:?}"),
    }
}

#[test]
fn splits_at_semicolons_and_at_line_breaks_between_complete_commands() {
    let cases: [(&str, &[&str]); 7] = [
        (
            "use shop; db.orders.find()",
            &["use shop", "db.orders.find()"],
        ),
        (
            "show dbs\nshow collections\n",
            &["show dbs", "show collections"],
        ),
        (
            "db.orders.find({\n  status: 'open',\n})\ndb.items.find()",
            &["db.orders.find({\n  status: 'open',\n})", "db.items.find()"],
        ),
        (
            "db.orders\n  .find()\n  .limit(5)\nshow dbs",
            &["db.orders\n  .find()\n  .limit(5)", "show dbs"],
        ),
        ("db.orders.\nfind()", &["db.orders.\nfind()"]),
        (
            "// first\ndb.a.find() /* inline */ ; ;\n\n// last\ndb.b.find()",
            &["db.a.find()", "db.b.find()"],
        ),
        ("", &[]),
    ];
    for (script, expected) in cases {
        assert_eq!(commands(script), expected, "{script:?}");
    }
}

#[test]
fn a_semicolon_inside_a_string_or_brackets_does_not_split() {
    assert_eq!(
        commands("db.a.find({x: \"a;b\"}); db.b.find()"),
        ["db.a.find({x: \"a;b\"})", "db.b.find()"]
    );
}

#[test]
fn tokens_cover_the_text_and_tell_names_strings_numbers_regexes_and_comments_apart() {
    let text = "db.a.find({n: -1.5e3, s: 'x\\'y', r: /a\\/b/i}) // done\n/* c */";
    let tokens = tokenize(text);
    let covered: String = tokens
        .iter()
        .map(|token| &text[token.range.clone()])
        .collect();
    assert_eq!(covered, text);
    let kinds: Vec<(TokenKind, &str)> = tokens
        .iter()
        .filter(|token| !matches!(token.kind, TokenKind::Whitespace | TokenKind::Punctuation))
        .map(|token| (token.kind, &text[token.range.clone()]))
        .collect();
    assert_eq!(
        kinds,
        [
            (TokenKind::Word, "db"),
            (TokenKind::Word, "a"),
            (TokenKind::Word, "find"),
            (TokenKind::Word, "n"),
            (TokenKind::Number, "1.5e3"),
            (TokenKind::Word, "s"),
            (TokenKind::String, "'x\\'y'"),
            (TokenKind::Word, "r"),
            (TokenKind::String, "/a\\/b/i"),
            (TokenKind::LineComment, "// done"),
            (TokenKind::BlockComment, "/* c */"),
        ]
    );
}

#[test]
fn reads_a_find_with_its_cursor_methods() {
    let found = call("db.orders.find({status: \"open\"}, {_id: 0}).sort({_id: -1}).limit(50)");
    assert_eq!(found.database, None);
    assert_eq!(found.collection.as_deref(), Some("orders"));
    assert_eq!(found.method.name, "find");
    assert_eq!(
        found.method.arguments,
        [
            object(&[("status", string("open"))]),
            object(&[("_id", number("0"))]),
        ]
    );
    let chain: Vec<(&str, &[Value])> = found
        .chain
        .iter()
        .map(|method| (method.name.as_str(), method.arguments.as_slice()))
        .collect();
    assert_eq!(
        chain,
        [
            ("sort", &[object(&[("_id", number("-1"))])][..]),
            ("limit", &[number("50")][..]),
        ]
    );
}

#[test]
fn reads_collections_by_dotted_name_through_get_collection_and_in_another_database() {
    assert_eq!(
        call("db.system.views.find()").collection.as_deref(),
        Some("system.views")
    );
    assert_eq!(
        call("db.getCollection('order lines').find()")
            .collection
            .as_deref(),
        Some("order lines")
    );
    let sibling = call("db.getSiblingDB(\"shop\").orders.countDocuments()");
    assert_eq!(sibling.database.as_deref(), Some("shop"));
    assert_eq!(sibling.collection.as_deref(), Some("orders"));
    let database = call("db.runCommand({ping: 1})");
    assert_eq!(database.collection, None);
    assert_eq!(database.method.name, "runCommand");
}

#[test]
fn reads_use_and_show() {
    assert_eq!(parse("use shop"), Ok(Command::Use("shop".to_owned())));
    assert_eq!(parse("use my-shop"), Ok(Command::Use("my-shop".to_owned())));
    assert_eq!(parse("use \"shop\""), Ok(Command::Use("shop".to_owned())));
    assert_eq!(parse("show dbs"), Ok(Command::Show(Show::Databases)));
    assert_eq!(parse("show databases"), Ok(Command::Show(Show::Databases)));
    assert_eq!(
        parse("show collections"),
        Ok(Command::Show(Show::Collections))
    );
    assert_eq!(parse("show tables"), Ok(Command::Show(Show::Collections)));
    assert!(parse("show users").is_err());
    assert!(parse("use").is_err());
}

/// The shell's relaxed JSON: unquoted keys, single quotes, trailing commas, comments, and
/// JavaScript's escapes.
#[test]
fn reads_relaxed_json() {
    let found = call(
        "db.a.insertOne({\n  // a comment\n  'quoted key': 'it\\'s',\n  \"a.b\": \"\\u00e9\\x41\\n\",\n  list: [1, 2,],\n  n: null, u: undefined, t: true, f: false,\n  big: 4294967296, neg: -0.5, inf: -Infinity, hex: 0x1f,\n})",
    );
    let [document] = found.method.arguments.as_slice() else {
        panic!("one argument expected");
    };
    assert_eq!(
        document,
        &object(&[
            ("quoted key", string("it's")),
            ("a.b", string("\u{e9}A\n")),
            ("list", Value::Array(vec![number("1"), number("2")])),
            ("n", Value::Null),
            ("u", Value::Undefined),
            ("t", Value::Bool(true)),
            ("f", Value::Bool(false)),
            ("big", number("4294967296")),
            ("neg", number("-0.5")),
            ("inf", number("-Infinity")),
            ("hex", number("0x1f")),
        ])
    );
}

#[test]
fn reads_the_constructors_of_bson_types() {
    let found = call(
        "db.a.find({a: ObjectId('507f1f77bcf86cd799439011'), b: ISODate(\"2026-10-31T00:00:00Z\"), c: new Date(0), d: NumberLong(\"9007199254740993\"), e: NumberDecimal('1.10'), f: UUID('0e3e1d4b-6c5d-4f6e-8a1b-2c3d4e5f6a7b'), g: Timestamp({t: 1, i: 2}), h: MinKey, i: MaxKey(), j: BinData(0, 'AQID'), k: /^ab/i})",
    );
    let fields = found.method.arguments[0].as_object().map(<[_]>::to_vec);
    let constructors: Vec<(String, Option<Constructor>)> = fields
        .unwrap_or_default()
        .into_iter()
        .map(|(key, value)| match value {
            Value::Construct { constructor, .. } => (key, Some(constructor)),
            _ => (key, None),
        })
        .collect();
    let expected: Vec<(String, Option<Constructor>)> = [
        ("a", Some(Constructor::ObjectId)),
        ("b", Some(Constructor::Date)),
        ("c", Some(Constructor::Date)),
        ("d", Some(Constructor::NumberLong)),
        ("e", Some(Constructor::NumberDecimal)),
        ("f", Some(Constructor::Uuid)),
        ("g", Some(Constructor::Timestamp)),
        ("h", Some(Constructor::MinKey)),
        ("i", Some(Constructor::MaxKey)),
        ("j", Some(Constructor::BinData)),
        ("k", None),
    ]
    .into_iter()
    .map(|(key, constructor)| (key.to_owned(), constructor))
    .collect();
    assert_eq!(constructors, expected);
    assert_eq!(
        call("db.a.find({k: /^ab/i})").method.arguments[0],
        object(&[(
            "k",
            Value::Regex {
                pattern: "^ab".to_owned(),
                flags: "i".to_owned()
            }
        )])
    );
}

/// Each error names what DbRover doesn't run and points at it.
#[test]
fn names_what_it_does_not_support_and_where() {
    let cases: [(&str, &str, &str); 9] = [
        (
            "const x = 1",
            "const",
            "\u{201c}const\u{201d} isn't supported",
        ),
        (
            "db.orders.find().forEach(printjson)",
            "forEach",
            "forEach() can't follow find()",
        ),
        (
            "db.orders.find({qty: limit})",
            "limit",
            "\u{201c}limit\u{201d} isn't a value",
        ),
        (
            "db.orders.find({$where: function() { return 1 }})",
            "function",
            "functions aren't supported",
        ),
        (
            "db.orders.find({x: (a) => a})",
            "(",
            "functions aren't supported",
        ),
        (
            "db.orders.mapReduce({})",
            "mapReduce",
            "mapReduce() isn't a collection method DbRover runs",
        ),
        (
            "db.orders.insertOne({_id: ObjectId('123')})",
            "ObjectId",
            "ObjectId() takes nothing, or 24 hexadecimal digits",
        ),
        (
            "db.orders.updateOne({_id: 1})",
            "updateOne",
            "updateOne() takes 2 to 3 arguments, not 1",
        ),
        (
            "printjson(db.orders.findOne())",
            "printjson",
            "\u{201c}printjson\u{201d} isn't supported",
        ),
    ];
    for (text, at, message) in cases {
        let error = error(text);
        assert!(
            error.message.starts_with(message),
            "{text}: {}",
            error.message
        );
        assert_eq!(&text[error.range.clone()], at, "{text}");
    }
}

#[test]
fn explain_comes_last() {
    let message = error("db.a.find().explain().limit(1)").message;
    assert_eq!(message, "explain() must come last");
}

#[test]
fn refuses_to_guess_what_text_without_exactly_one_command_does() {
    assert_eq!(classify("  // nothing\n"), Err(Unclassified::Empty));
    assert_eq!(
        classify("db.a.find()\ndb.b.find()"),
        Err(Unclassified::Several(2))
    );
}

/// Each command in `classify.txt` follows a comment naming what it must be classified as,
/// `// expect: read` (or `write`, `schema`, `admin`, `unparseable`), as the other languages'
/// corpora do.
#[test]
fn classifies_its_corpus() {
    const EXPECT: &str = "// expect:";
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
    assert!(checked > 70, "only {checked} commands checked");
}

#[test]
fn warns_of_commands_that_may_touch_everything() {
    let cases: [(&str, bool); 8] = [
        ("db.orders.deleteMany({})", true),
        ("db.orders.deleteMany({status: 'closed'})", false),
        ("db.orders.remove({})", true),
        ("db.orders.updateMany({}, {$set: {a: 1}})", true),
        ("db.orders.updateMany({a: 1}, {$set: {a: 2}})", false),
        ("db.orders.drop()", true),
        ("db.dropDatabase()", true),
        ("db.orders.find({})", false),
    ];
    for (command, warns) in cases {
        assert_eq!(hazard(command).is_some(), warns, "{command}");
    }
}

#[test]
fn completes_methods_operators_and_stages_with_what_each_is() {
    let words = completion_words();
    assert!(words.is_sorted());
    for word in ["find", "insertOne", "$match", "$set", "ObjectId", "limit"] {
        assert!(words.contains(&word), "{word}");
    }
    assert_eq!(syntax("updateOne"), Some("filter, update, options"));
    assert_eq!(syntax("$match"), Some("pipeline stage"));
    assert_eq!(syntax("$gt"), Some("query operator"));
    assert_eq!(syntax("getIndexes"), None);
    assert!(is_keyword("find"));
    assert!(is_keyword("ObjectId"));
    assert!(!is_keyword("FIND"));
    assert!(!is_keyword("orders"));
}

#[test]
fn open_data_finds_every_document_of_any_collection() {
    assert_eq!(
        find_all("shop", "orders", "shop"),
        "db.getCollection(\"orders\").find({})"
    );
    let elsewhere = find_all("logs", "web \"2026\".v1", "shop");
    assert_eq!(
        elsewhere,
        "db.getSiblingDB(\"logs\").getCollection(\"web \\\"2026\\\".v1\").find({})"
    );
    let Ok(Command::Call(call)) = parse(&elsewhere) else {
        panic!("{elsewhere} doesn't parse");
    };
    assert_eq!(call.database.as_deref(), Some("logs"));
    assert_eq!(call.collection.as_deref(), Some("web \"2026\".v1"));
    assert!(is_plain_name("$match"));
    assert!(!is_plain_name("a.b"));
    assert!(!is_plain_name(""));
}

#[test]
fn explains_finds_and_aggregations_only() {
    assert_eq!(
        explain("db.orders.find({a: 1}).sort({b: 1})").as_deref(),
        Some("db.orders.find({a: 1}).sort({b: 1}).explain(\"executionStats\")")
    );
    assert!(explain("db.orders.aggregate([])").is_some());
    assert_eq!(explain("db.orders.find().explain()"), None);
    assert_eq!(explain("db.orders.deleteMany({})"), None);
    assert!(returns_documents("db.orders.find()"));
    assert!(!returns_documents("db.orders.countDocuments()"));
}

mod edits {
    use dbrover_document::{Path, Step, Value as Document};

    use super::super::*;

    fn document(json: &str) -> Document {
        Document::parse(json).unwrap()
    }

    /// Text typed over a field keeps the type the field had, read from its Extended JSON.
    #[test]
    fn typed_text_keeps_the_old_values_type() {
        let cases: [(&str, Option<&str>, Result<&str, ()>); 14] = [
            (
                r#"{"$numberLong": "7"}"#,
                Some("8"),
                Ok(r#"NumberLong("8")"#),
            ),
            (r#"{"$numberInt": "7"}"#, Some("8"), Ok("NumberInt(8)")),
            (r#"{"$numberInt": "7"}"#, Some("abc"), Err(())),
            (r#"{"$numberDouble": "1.5"}"#, Some("2"), Ok("2.0")),
            (
                r#"{"$numberDecimal": "1.10"}"#,
                Some("2.20"),
                Ok(r#"NumberDecimal("2.20")"#),
            ),
            (r#""Madrid""#, Some("8"), Ok(r#""8""#)),
            (r#""Madrid""#, Some("say \"hi\""), Ok(r#""say \"hi\"""#)),
            ("true", Some("false"), Ok("false")),
            (
                r#"{"$date": {"$numberLong": "0"}}"#,
                Some("2026-10-31T00:00:00Z"),
                Ok(r#"ISODate("2026-10-31T00:00:00Z")"#),
            ),
            (
                r#"{"$oid": "507f1f77bcf86cd799439011"}"#,
                Some(r#"ObjectId("507f1f77bcf86cd799439012")"#),
                Ok(r#"ObjectId("507f1f77bcf86cd799439012")"#),
            ),
            (r#"{"a": 1}"#, Some("{a: 2, b: [1]}"), Ok("{a: 2, b: [1]}")),
            ("null", Some("42"), Ok("42")),
            ("null", Some("hello"), Ok(r#""hello""#)),
            (r#"{"$numberLong": "7"}"#, None, Ok("null")),
        ];
        for (old, typed, expected) in cases {
            let old = document(old);
            let written = typed_value(Some(&old), typed).map_err(|_| ());
            assert_eq!(
                written.as_deref(),
                expected.as_deref(),
                "{typed:?} over {old:?}"
            );
        }
        assert_eq!(typed_value(None, Some("3")).as_deref(), Ok("3"));
    }

    /// A value written back in the shell's syntax reads as the same Extended JSON value.
    #[test]
    fn writes_each_value_in_the_shells_syntax() {
        let value = document(concat!(
            r#"{"_id": {"$oid": "507f1f77bcf86cd799439011"}, "n": {"$numberLong": "9007199254740993"}, "#,
            r#""at": {"$date": {"$numberLong": "1793439000123"}}, "tags": ["a", {"$numberInt": "1"}], "#,
            r#""first name": "Ada", "bin": {"$binary": {"base64": "AQID", "subType": "00"}}}"#
        ));
        assert_eq!(
            shell_value(&value).as_deref(),
            Ok(concat!(
                r#"{_id: ObjectId("507f1f77bcf86cd799439011"), n: NumberLong(9007199254740993), "#,
                r#"at: ISODate("2026-10-31T09:30:00.123Z"), tags: ["a", 1], "#,
                r#""first name": "Ada", bin: BinData(0, "AQID")}"#
            ))
        );
        assert_eq!(
            document_id(&value).as_deref(),
            Ok(r#"ObjectId("507f1f77bcf86cd799439011")"#)
        );
        assert!(document_id(&document(r#"{"a": 1}"#)).is_err());
        assert!(shell_value(&document(r#"{"$code": "x"}"#)).is_err());
    }

    #[test]
    fn edits_become_commands_on_the_collection_found() {
        let view = CollectionView::of("db.getSiblingDB('logs').events.find({a: 1})").unwrap();
        assert_eq!(view.collection(), "events");
        assert_eq!(
            view.update("1", "address.city", Some("\"Madrid\"")),
            r#"db.getSiblingDB("logs").getCollection("events").updateOne({_id: 1}, {$set: {"address.city": "Madrid"}})"#
        );
        assert_eq!(
            view.update("1", "gone", None),
            r#"db.getSiblingDB("logs").getCollection("events").updateOne({_id: 1}, {$unset: {"gone": ""}})"#
        );
        let orders = CollectionView::of("db.orders.find()").unwrap();
        assert_eq!(
            orders.delete("2"),
            r#"db.getCollection("orders").deleteOne({_id: 2})"#
        );
        assert_eq!(
            orders.insert("{a: 1}"),
            r#"db.getCollection("orders").insertOne({a: 1})"#
        );
        for command in [
            orders.update("1", "a", Some("2")),
            orders.delete("1"),
            orders.insert("{a: 1}"),
        ] {
            assert!(parse(&command).is_ok(), "{command}");
        }
        assert_eq!(CollectionView::of("db.orders.aggregate([])"), None);
        assert_eq!(CollectionView::of("db.orders.find().explain()"), None);
        assert_eq!(
            update_path(&Path::from(vec![
                Step::Field("items".to_owned()),
                Step::Index(2),
                Step::Field("sku".to_owned()),
            ]))
            .as_deref(),
            Ok("items.2.sku")
        );
        assert!(update_path(&Path::from(vec![Step::Field("a.b".to_owned())])).is_err());
    }
}
