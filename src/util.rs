pub struct DropAction<F: Fn()>(pub F);

impl<F: Fn()> Drop for DropAction<F> {
    fn drop(&mut self) {
        (self.0)()
    }
}
