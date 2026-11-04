use jit::search::{search, SearchOptions};
use jit::test_cli::JitCli;
use jit::test_taxonomy::setup_test_repo_with_taxonomy;
use std::fs;

/// Seed three issues and one design document into `cli`.
fn seed_searchable_content(cli: &JitCli) {
    [
        (
            "Implement user authentication",
            "Add JWT authentication middleware",
            "high",
        ),
        (
            "Add rate limiting",
            "Prevent brute force attacks",
            "critical",
        ),
        (
            "Write documentation",
            "Document the API endpoints",
            "normal",
        ),
    ]
    .into_iter()
    .for_each(|(title, description, priority)| {
        cli.create_issue([
            "--title",
            title,
            "--description",
            description,
            "--priority",
            priority,
        ])
        .unwrap();
    });

    let docs_dir = cli.root().join(".jit/docs");
    fs::create_dir_all(&docs_dir).unwrap();
    fs::write(
        docs_dir.join("design.md"),
        "# Authentication Design\n\nThe authentication flow uses OAuth 2.0 with PKCE.\n",
    )
    .unwrap();
}

#[test]
fn test_search_finds_issue_by_title() {
    let cli = JitCli::initialised().unwrap();
    seed_searchable_content(&cli);
    let jit_dir = cli.root().join(".jit");

    let results = search(&jit_dir, "authentication", SearchOptions::default()).unwrap();

    assert!(!results.is_empty());
    let auth_results: Vec<_> = results
        .iter()
        .filter(|r| r.line_text.contains("authentication"))
        .collect();
    assert!(!auth_results.is_empty());
}

#[test]
fn test_search_finds_issue_by_description() {
    let cli = JitCli::initialised().unwrap();
    seed_searchable_content(&cli);
    let jit_dir = cli.root().join(".jit");

    let results = search(&jit_dir, "JWT", SearchOptions::default()).unwrap();

    assert!(!results.is_empty());
    let jwt_result = results.iter().find(|r| r.line_text.contains("JWT"));
    assert!(jwt_result.is_some());
}

#[test]
fn test_search_with_regex() {
    let cli = JitCli::initialised().unwrap();
    seed_searchable_content(&cli);
    let jit_dir = cli.root().join(".jit");

    let options = SearchOptions {
        regex: true,
        ..Default::default()
    };

    let results = search(&jit_dir, "auth(entication|orization)", options).unwrap();
    assert!(!results.is_empty());
}

#[test]
fn test_search_with_glob_filter_json_only() {
    let cli = JitCli::initialised().unwrap();
    seed_searchable_content(&cli);
    let jit_dir = cli.root().join(".jit");

    let options = SearchOptions {
        file_pattern: Some("*.json".to_string()),
        ..Default::default()
    };

    let results = search(&jit_dir, "authentication", options).unwrap();

    // Should find in JSON files
    assert!(!results.is_empty());
    // All results should be from .json files
    for result in &results {
        assert!(result.path.ends_with(".json"));
    }
}

#[test]
fn test_search_with_glob_filter_markdown_only() {
    let cli = JitCli::initialised().unwrap();
    seed_searchable_content(&cli);
    let jit_dir = cli.root().join(".jit");

    let options = SearchOptions {
        file_pattern: Some("*.md".to_string()),
        ..Default::default()
    };

    let results = search(&jit_dir, "OAuth", options).unwrap();

    // Should find in markdown files
    assert!(!results.is_empty());
    // All results should be from .md files
    for result in &results {
        assert!(result.path.ends_with(".md"));
    }
}

#[test]
fn test_search_case_sensitive() {
    let cli = JitCli::initialised().unwrap();
    seed_searchable_content(&cli);
    let jit_dir = cli.root().join(".jit");

    let case_insensitive = SearchOptions {
        case_sensitive: false,
        ..Default::default()
    };
    let results_insensitive = search(&jit_dir, "AUTHENTICATION", case_insensitive).unwrap();

    let case_sensitive = SearchOptions {
        case_sensitive: true,
        ..Default::default()
    };
    let results_sensitive = search(&jit_dir, "AUTHENTICATION", case_sensitive).unwrap();

    // Case-insensitive should find matches
    assert!(!results_insensitive.is_empty());
    // Case-sensitive should not find matches (all lowercase in files)
    assert!(results_sensitive.is_empty());
}

#[test]
fn test_search_limit_results() {
    let cli = JitCli::initialised().unwrap();
    seed_searchable_content(&cli);
    let jit_dir = cli.root().join(".jit");

    let options = SearchOptions {
        max_results: Some(2),
        ..Default::default()
    };

    let results = search(&jit_dir, "priority", options).unwrap();

    // Should respect the limit
    assert!(results.len() <= 2);
}

#[test]
fn test_search_no_matches() {
    let cli = JitCli::initialised().unwrap();
    seed_searchable_content(&cli);
    let jit_dir = cli.root().join(".jit");

    let results = search(&jit_dir, "nonexistent_term_xyz", SearchOptions::default()).unwrap();

    assert!(results.is_empty());
}

#[test]
fn test_search_in_documents() {
    let cli = JitCli::initialised().unwrap();
    seed_searchable_content(&cli);
    let jit_dir = cli.root().join(".jit");

    let results = search(&jit_dir, "PKCE", SearchOptions::default()).unwrap();

    assert!(!results.is_empty());
    let doc_result = results.iter().find(|r| r.path.contains("design.md"));
    assert!(doc_result.is_some());
    // Document paths should not have issue IDs
    assert_eq!(doc_result.unwrap().issue_id, None);
}

#[test]
fn test_search_extracts_issue_ids() {
    let cli = JitCli::initialised().unwrap();
    seed_searchable_content(&cli);
    let jit_dir = cli.root().join(".jit");

    let results = search(&jit_dir, "priority", SearchOptions::default()).unwrap();

    // Find results from issue files
    let issue_results: Vec<_> = results
        .iter()
        .filter(|r| r.path.contains("issues/"))
        .collect();

    assert!(!issue_results.is_empty());

    // All issue files should have extracted IDs
    for result in issue_results {
        assert!(result.issue_id.is_some());
        // ID should match filename pattern
        let id = result.issue_id.as_ref().unwrap();
        assert!(result.path.contains(id.as_str()));
    }
}

#[test]
fn test_search_json_omits_total() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    cli.create_issue([
        "--title",
        "Implement user authentication",
        "--description",
        "Add JWT authentication middleware",
    ])
    .unwrap();

    let json = cli.run_json(["search", "authentication"]).unwrap();

    assert!(
        json.get("total").is_none(),
        "top-level search JSON carries no total: {json}"
    );
}
