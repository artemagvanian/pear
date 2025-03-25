use std::collections::LinkedList;

#[pear::scrutinizer_pure]
pub fn contains_linked_list(haystack: &LinkedList<usize>, needle: &usize) -> bool {
    haystack.contains(needle)
}
