//! Fuzzy ranking for palettes and switchers. The palette never filters (the app owns its rows);
//! this is what an app filters with: `fuzzy::rank(query, notes, |n| n.title.as_ref())`.
//!
//! A query matches when its characters appear in order in the key, ignoring case. Matches score
//! higher for a prefix, for characters at word starts, and for runs of consecutive characters;
//! equal scores keep the input order. An empty query keeps every item in order.

/// The items whose key matches `query`, best first.
pub fn rank<T>(query: &str, items: impl IntoIterator<Item = T>, key: impl Fn(&T) -> &str) -> std::vec::IntoIter<T> {
    let query: Vec<char> = query.chars().flat_map(char::to_lowercase).filter(|c| !c.is_whitespace()).collect();
    let mut scored: Vec<(i64, usize, T)> =
        items.into_iter().enumerate().filter_map(|(i, item)| score(&query, key(&item)).map(|s| (s, i, item))).collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, _, item)| item).collect::<Vec<_>>().into_iter()
}

/// How well `query` (lowercase, no spaces) matches `text`; `None` when it does not.
pub fn score(query: &[char], text: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(0);
    }
    let chars: Vec<char> = text.chars().collect();
    let lower: Vec<char> = chars.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect();
    let mut score = 0i64;
    let mut at = 0;
    let mut previous: Option<usize> = None;
    for &q in query {
        let found = (at..lower.len()).find(|&i| lower[i] == q)?;
        let word_start = found == 0 || !chars[found - 1].is_alphanumeric();
        score += 1;
        if word_start {
            score += 8;
        }
        if previous.is_some_and(|p| p + 1 == found) {
            score += 5;
        }
        // Gaps cost a little, so tighter matches win.
        score -= (found - previous.map_or(0, |p| p + 1)).min(10) as i64;
        previous = Some(found);
        at = found + 1;
    }
    let prefix = lower.iter().zip(query).take_while(|(a, b)| a == b).count();
    if prefix == query.len() {
        score += 20;
    }
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranked(query: &str, items: &[&'static str]) -> Vec<&'static str> {
        rank(query, items.iter().copied(), |s| s).collect()
    }

    #[test]
    fn an_empty_query_keeps_everything_in_order() {
        assert_eq!(ranked("", &["b", "a"]), ["b", "a"]);
    }

    #[test]
    fn characters_in_order_ignoring_case() {
        assert_eq!(ranked("tl", &["Trip to Lisbon", "Groceries", "Lt"]), ["Trip to Lisbon"]);
    }

    #[test]
    fn prefixes_and_word_starts_rank_first() {
        assert_eq!(
            ranked("to", &["Photo", "Toggle sidebar", "Trip to Lisbon"]),
            ["Toggle sidebar", "Trip to Lisbon", "Photo"]
        );
        assert_eq!(ranked("ts", &["Tags", "Toggle sidebar"]), ["Toggle sidebar", "Tags"], "word starts beat a gap");
    }

    #[test]
    fn equal_scores_keep_input_order() {
        assert_eq!(ranked("a", &["ab", "ac"]), ["ab", "ac"]);
    }
}
