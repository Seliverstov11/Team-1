//! Настройки из переменных окружения. Все необязательные: без них проект
//! запускается по README на чистой машине (H6). Секреты — только в .env (H12).

use std::sync::OnceLock;

pub struct Config {
    /// Оптовые цены. Заказчик хочет убрать опт, цена одна для всех
    /// (встреча 3, 27:47). Пока он не подтвердил окончательно, код опта
    /// остаётся, но по умолчанию выключен: на сайте и в заказах опта нет.
    pub wholesale_enabled: bool,
    /// Telegram-уведомления сотрудникам о новых заказах (ФТ-М-13).
    /// Без токена и чата уведомления просто не отправляются.
    pub telegram_token: Option<String>,
    pub telegram_chat_id: Option<String>,
    /// Реквизиты для накладной.
    pub shop_name: String,
    pub shop_bin: Option<String>,
    pub shop_address: Option<String>,
    /// Сколько живёт вход.
    pub session_days: i64,
}

fn non_empty(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

pub fn get() -> &'static Config {
    static CFG: OnceLock<Config> = OnceLock::new();
    CFG.get_or_init(|| Config {
        wholesale_enabled: matches!(
            non_empty("WHOLESALE_ENABLED").as_deref(),
            Some("1" | "true" | "yes")
        ),
        telegram_token: non_empty("TELEGRAM_BOT_TOKEN"),
        telegram_chat_id: non_empty("TELEGRAM_CHAT_ID"),
        shop_name: non_empty("SHOP_NAME").unwrap_or_else(|| "BAKELOOM".into()),
        shop_bin: non_empty("SHOP_BIN"),
        shop_address: non_empty("SHOP_ADDRESS"),
        session_days: non_empty("SESSION_DAYS")
            .and_then(|v| v.parse().ok())
            .unwrap_or(30),
    })
}
