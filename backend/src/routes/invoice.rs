//! Накладная по заказу (ФТ-М-12). Форма — «Накладная на отпуск запасов на
//! сторону» (З-2), по которой работают магазины в Казахстане. Заказчик
//! обещал прислать свой образец (В3 04:39) — поля сверим с ним.
//! Отдаётся HTML-страницей: её печатают из браузера или сохраняют в PDF.

use axum::{
    extract::{Path, State},
    response::Html,
};

use crate::{
    auth::{ANY_STAFF, Staff},
    db::Db,
    error::ApiResult,
    num::tiyn_to_string,
};

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub async fn invoice(staff: Staff, State(db): State<Db>, Path(id): Path<String>) -> ApiResult<Html<String>> {
    staff.require(ANY_STAFF)?;
    let conn = db.connect()?;
    let o = super::orders::load_order(&conn, &id).await?;
    let items = super::orders::load_items(&conn, &id).await?;
    let cfg = crate::config::get();
    let date = o.created_at.get(..10).unwrap_or("").split('-').rev().collect::<Vec<_>>().join(".");
    let show_money = !staff.is(crate::auth::Role::Warehouse);

    let mut rows = String::new();
    for (n, i) in items.iter().enumerate() {
        let (price, sum) = if show_money {
            (tiyn_to_string(i.price_tiyn), tiyn_to_string(i.subtotal_tiyn))
        } else {
            (String::new(), String::new())
        };
        rows.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td class=n>{}</td><td class=n>{}</td><td class=n>{}</td><td class=n>{}</td></tr>",
            n + 1,
            esc(&i.product_name),
            esc(i.product_sku.as_deref().unwrap_or("")),
            esc(&i.unit),
            crate::num::milli_to_string(i.quantity_milli),
            crate::num::milli_to_string(i.quantity_milli - i.short_milli),
            price,
            sum
        ));
    }
    let totals = if show_money {
        format!(
            "<tr><td colspan=7 class=r>Товары</td><td class=n>{}</td></tr>
             <tr><td colspan=7 class=r>Доставка</td><td class=n>{}</td></tr>
             <tr class=b><td colspan=7 class=r>Итого, ₸</td><td class=n>{}</td></tr>",
            tiyn_to_string(o.items_tiyn),
            tiyn_to_string(o.delivery_tiyn),
            tiyn_to_string(o.total_tiyn)
        )
    } else {
        String::new()
    };
    let qty_total: i64 = items.iter().map(|i| i.quantity_milli).sum();

    Ok(Html(format!(
        r#"<!doctype html><html lang=ru><head><meta charset=utf-8>
<title>Накладная № {no}</title>
<style>
 body{{font:13px/1.4 Arial,sans-serif;color:#000;margin:24px}}
 h1{{font-size:16px;text-align:center;margin:16px 0 4px}}
 .sub{{text-align:center;margin-bottom:16px}}
 table{{border-collapse:collapse;width:100%}} th,td{{border:1px solid #000;padding:4px 6px;vertical-align:top}}
 th{{font-weight:normal;background:#f2f2f2}} .n{{text-align:right;white-space:nowrap}} .r{{text-align:right}} .b td{{font-weight:bold}}
 .meta td{{border:none;padding:2px 6px}} .sign{{margin-top:28px;display:grid;grid-template-columns:1fr 1fr;gap:20px 40px}}
 .sign div{{border-top:1px solid #000;padding-top:2px;font-size:11px}} .form{{text-align:right;font-size:11px}}
 @media print{{body{{margin:10mm}}}}
</style></head><body>
<div class=form>Форма З-2</div>
<table class=meta>
 <tr><td>Организация (отправитель):</td><td><b>{shop}</b>{bin}</td></tr>
 <tr><td>Адрес:</td><td>{addr}</td></tr>
 <tr><td>Получатель:</td><td><b>{client}</b>, тел. {phone}</td></tr>
 <tr><td>Получение:</td><td>{method}{city}{daddr}</td></tr>
</table>
<h1>НАКЛАДНАЯ НА ОТПУСК ЗАПАСОВ НА СТОРОНУ № {no}</h1>
<div class=sub>от {date}</div>
<table>
 <tr><th>№</th><th>Наименование</th><th>Номенклатурный номер</th><th>Ед. изм.</th><th>Подлежит отпуску</th><th>Отпущено</th><th>Цена, ₸</th><th>Сумма, ₸</th></tr>
 {rows}
 {totals}
</table>
<p>Всего отпущено количество: {qty}</p>
<div class=sign>
 <div>Отпуск разрешил (должность, подпись, расшифровка)</div>
 <div>Получил (подпись, расшифровка)</div>
 <div>Отпустил: {who}</div>
 <div>Дата получения</div>
</div>
</body></html>"#,
        no = esc(o.order_no.as_deref().unwrap_or(&o.id)),
        shop = esc(&cfg.shop_name),
        bin = cfg.shop_bin.as_deref().map(|b| format!(", БИН {}", esc(b))).unwrap_or_default(),
        addr = esc(cfg.shop_address.as_deref().unwrap_or("—")),
        client = esc(&o.client_name),
        phone = esc(o.client_phone.as_deref().unwrap_or("—")),
        method = esc(match o.delivery_method.as_str() {
            "pickup" => "самовывоз",
            "courier" => "курьер",
            "cdek" => "СДЭК",
            "kazpost" => "Казпочта",
            m => m,
        }),
        city = o.city.as_deref().map(|c| format!(", {}", esc(c))).unwrap_or_default(),
        daddr = o.delivery_address.as_deref().map(|a| format!(", {}", esc(a))).unwrap_or_default(),
        date = esc(&date),
        rows = rows,
        totals = totals,
        qty = crate::num::milli_to_string(qty_total),
        who = esc(&staff.name),
    )))
}
