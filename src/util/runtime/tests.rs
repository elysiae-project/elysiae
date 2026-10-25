use super::*;

#[test]
fn runtime_is_initialized_once_and_executes_tasks() {
    let first = runtime();
    let second = runtime();

    assert!(std::ptr::eq(first, second));
    assert_eq!(first.block_on(async { 2 + 2 }), 4);
}
