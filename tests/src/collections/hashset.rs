use std::collections::HashSet;

fn contains_hashset(haystack: &HashSet<usize>, needle: &usize) -> bool {
    haystack.contains(needle)
}
