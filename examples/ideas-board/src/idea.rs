// tag::entity[]
/// An idea on the board: an id, its title, and a running vote count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Idea {
    pub id: String,
    pub title: String,
    pub votes: i64,
}
// end::entity[]
