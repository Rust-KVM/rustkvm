use salvo::prelude::*;

#[endpoint]
pub(super) async fn handle_pprof_index() -> &'static str {
    "rustkvm pprof not implemented"
}

#[endpoint]
pub(super) async fn handle_pprof_cmdline() -> &'static str {
    "rustkvm cmdline not implemented"
}

#[endpoint]
pub(super) async fn handle_pprof_profile() -> &'static str {
    "rustkvm profile not implemented"
}

#[endpoint]
pub(super) async fn handle_pprof_symbol() -> &'static str {
    "rustkvm symbol not implemented"
}

#[endpoint]
pub(super) async fn handle_pprof_create_symbol() -> &'static str {
    "rustkvm symbol creation not implemented"
}

#[endpoint]
pub(super) async fn handle_pprof_trace() -> &'static str {
    "rustkvm trace not implemented"
}

#[endpoint]
pub(super) async fn handle_pprof_allocs() -> &'static str {
    "rustkvm allocs not implemented"
}

#[endpoint]
pub(super) async fn handle_pprof_block() -> &'static str {
    "rustkvm block not implemented"
}

#[endpoint]
pub(super) async fn handle_pprof_goroutine() -> &'static str {
    "rustkvm goroutine not implemented"
}

#[endpoint]
pub(super) async fn handle_pprof_heap() -> &'static str {
    "rustkvm heap not implemented"
}

#[endpoint]
pub(super) async fn handle_pprof_mutex() -> &'static str {
    "rustkvm mutex not implemented"
}

#[endpoint]
pub(super) async fn handle_pprof_threadcreate() -> &'static str {
    "rustkvm threadcreate not implemented"
}
