fn wind_pr_default_trace(issue: u64) -> String {
    format!("Closes #{issue}\nREQ: #{issue}")
}

#[cfg(test)]
mod wind_pr_trace_tests {
    use super::*;

    #[test]
    fn bound_intake_issue_is_the_default_req_line() {
        let delivery = DeliveryEvidence {
            version: 1,
            issue: 42,
            mode: "standard".to_owned(),
            risk_tags: Vec::new(),
            engine: "omx".to_owned(),
            spec: None,
            verification: DeliveryVerification {
                commands: Vec::new(),
                live_evidence: "VERIFIED-LIVE: trace rendering".to_owned(),
                artifacts: Vec::new(),
                open_risks: Vec::new(),
            },
        };

        assert!(
            pr_render_delivery_body(None, &delivery).contains("Closes #42\nREQ: #42"),
            "bound intake issue must replace the historical REQ: none default"
        );
    }
}
