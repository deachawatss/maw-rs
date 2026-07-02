use extism_pdk::*;

#[plugin_fn]
pub fn handle(_input: String) -> FnResult<String> {
    Ok("{\"ok\":true,\"output\":\"route-probe:called\"}".to_owned())
}
