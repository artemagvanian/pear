use std::collections::HashSet;

#[pear::scrutinizer_pure]
pub fn contains_hashset(haystack: &HashSet<usize>, needle: &usize) -> bool {
    haystack.contains(needle)
}
