fn main() {
    let result = notify_rust::Notification::new()
        .summary("Terminal Workspace")
        .body("실시간 검증: 이 토스트가 보이면 데스크톱 알림이 실제로 동작하는 것입니다.")
        .show();
    match result {
        Ok(_) => println!("NOTIFY_OK"),
        Err(e) => println!("NOTIFY_ERR: {e}"),
    }
}
