pub use firm_protocols::*;

pub mod stream;

pub struct Displayer<'a, T> {
    display: &'a T,
}

impl<T> std::ops::Deref for Displayer<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.display
    }
}

pub trait DisplayExt<'a, T> {
    fn display(&'a self) -> Displayer<T>;
}

impl<'a, U> DisplayExt<'a, U> for U {
    fn display(&'a self) -> Displayer<U> {
        Displayer { display: self }
    }
}
