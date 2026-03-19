pub trait ToolAction {
    fn run(&self) -> Result<(), String>;
}
