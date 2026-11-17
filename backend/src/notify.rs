//! Уведомление сотрудникам в Telegram о новом заказе, в формате накладной
//! (ФТ-М-13). Это первая из двух разрешённых интеграций (H9), бесплатная.
//! Покупателю такие уведомления не нужны — он смотрит заказ на сайте.
//!
//! Без TELEGRAM_BOT_TOKEN и TELEGRAM_CHAT_ID ничего не отправляется.
//! Сбой Telegram не мешает оформлению: заказ уже сохранён.

use crate::models::OrderDetail;

fn delivery_title(m: &str) -> &'static str {
    match m {
        "pickup" => "самовывоз",
        "courier" => "курьер",
        "cdek" => "СДЭК",
        "kazpost" => "Казпочта",
        _ => "—",
    }
}

pub fn order_text(d: &OrderDetail) -> String {
    let o = &d.order;
    let mut t = format!(
        "🧾 Новый заказ №{}\nКлиент: {}\nТелефон: {}\nКанал: {}\nПолучение: {}",
        o.order_no.as_deref().unwrap_or("—"),
        o.client_name,
        o.client_phone.as_deref().unwrap_or("—"),
        o.channel,
        delivery_title(&o.delivery_method),
    );
    if let Some(c) = &o.city {
        t.push_str(&format!(", {c}"));
    }
    if let Some(a) = &o.delivery_address {
        t.push_str(&format!(", {a}"));
    }
    t.push_str("\n\n");
    for (n, i) in d.items.iter().enumerate() {
        t.push_str(&format!(
            "{}. {} — {} {} × {} = {} ₸\n",
            n + 1,
            i.product_name,
            i.quantity,
            i.unit,
            i.price,
            i.subtotal
        ));
    }
    t.push_str(&format!("\nТовары: {} ₸\nДоставка: {} ₸\nИтого: {} ₸", o.items_total, o.delivery, o.total));
    if o.is_urgent {
        t.push_str("\n⚡ Срочный");
    }
    if let Some(n) = &o.note {
        t.push_str(&format!("\nКомментарий: {n}"));
    }
    t
}

pub fn order_created(d: &OrderDetail) {
    let cfg = crate::config::get();
    let (Some(token), Some(chat)) = (cfg.telegram_token.clone(), cfg.telegram_chat_id.clone()) else {
        return;
    };
    let text = order_text(d);
    let no = d.order.order_no.clone().unwrap_or_default();
    tokio::spawn(async move {
        let url = format!("https://api.telegram.org/bot{token}/sendMessage");
        let res = reqwest::Client::new()
            .post(url)
            .json(&serde_json::json!({ "chat_id": chat, "text": text }))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await;
        match res {
            Ok(r) if r.status().is_success() => tracing::info!("Telegram: заказ №{no} отправлен"),
            Ok(r) => tracing::warn!("Telegram ответил {} на заказ №{no}", r.status()),
            Err(e) => tracing::warn!("Telegram недоступен, заказ №{no}: {e}"),
        }
    });
}
