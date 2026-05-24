pub trait Diffable {
    type Diff;
    fn diff(&self, target: &Self) -> Self::Diff;

    fn apply(&mut self, diff: Self::Diff);
}
