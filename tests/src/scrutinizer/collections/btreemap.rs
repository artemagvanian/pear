use std::collections::BTreeMap;

#[pear::scrutinizer_pure]
pub fn contains_btreemap(haystack: &BTreeMap<usize, usize>, needle: &usize) -> bool {
    haystack.contains_key(needle)
}
