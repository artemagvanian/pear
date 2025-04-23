mod async_fns {
    use futures::executor::block_on;

    #[pear::analysis_entry]
    async fn one_level_async() {
        println!("hello, world!");
    }

    #[pear::analysis_entry]
    fn non_async() {
        let future = one_level_async(); 
        block_on(future);
    }

    #[pear::analysis_entry]
    async fn two_levels_async() {
        one_level_async().await;
    }
}