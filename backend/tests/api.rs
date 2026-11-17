//! Тесты API на настоящем роутере и временной БД с демо-данными.

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode},
};
use bakeloom_api::db::Db;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const PW: &str = "test-pass";

struct T {
    app: Router,
    db: Db,
}

async fn setup() -> T {
    let path = std::env::temp_dir().join(format!("bakeloom-test-{}.db", uuid::Uuid::new_v4()));
    let db = bakeloom_api::db::connect_local(path.to_str().unwrap()).await.unwrap();
    let hash = bakeloom_api::auth::hash_password(PW).await.unwrap();
    db.connect().unwrap().execute("UPDATE employees SET password_hash = ?1", libsql::params![hash]).await.unwrap();
    T { app: bakeloom_api::app(db.clone()), db }
}

impl T {
    async fn call(&self, method: Method, uri: &str, token: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
        let mut b = Request::builder().method(method).uri(uri).header("content-type", "application/json");
        if let Some(t) = token {
            b = b.header("authorization", format!("Bearer {t}"));
        }
        let req = b.body(body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty)).unwrap();
        let res = self.app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8_lossy(&bytes).to_string();
        (status, serde_json::from_str(&text).unwrap_or(Value::String(text)))
    }
    async fn get(&self, uri: &str, token: Option<&str>) -> (StatusCode, Value) {
        self.call(Method::GET, uri, token, None).await
    }
    async fn post(&self, uri: &str, token: Option<&str>, body: Value) -> (StatusCode, Value) {
        self.call(Method::POST, uri, token, Some(body)).await
    }
    async fn patch(&self, uri: &str, token: Option<&str>, body: Value) -> (StatusCode, Value) {
        self.call(Method::PATCH, uri, token, Some(body)).await
    }
    async fn login(&self, login: &str) -> String {
        let (s, v) = self.post("/api/auth/login", None, json!({ "login": login, "password": PW })).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v["token"].as_str().unwrap().to_string()
    }
    async fn one(&self, sql: &str) -> String {
        let conn = self.db.connect().unwrap();
        let mut r = conn.query(sql, ()).await.unwrap();
        r.next().await.unwrap().unwrap().get::<String>(0).unwrap()
    }
    async fn num(&self, sql: &str) -> i64 {
        let conn = self.db.connect().unwrap();
        let mut r = conn.query(sql, ()).await.unwrap();
        r.next().await.unwrap().unwrap().get::<i64>(0).unwrap()
    }
    /// Штучный товар, которого на складе хватает с запасом.
    async fn product(&self) -> String {
        self.one(
            "SELECT id FROM products WHERE stock_milli - reserved_milli >= 20000
               AND min_order_milli = 1000 AND order_step_milli = 1000 AND is_archived = 0
             ORDER BY id LIMIT 1",
        )
        .await
    }
    async fn guest_order(&self, product: &str, qty: &str) -> Value {
        let (s, v) = self
            .post(
                "/api/orders",
                None,
                json!({ "client_name": "Тест Покупатель", "client_phone": "+7 701 555 00 11",
                        "delivery_method": "courier", "city": "Астана", "delivery_address": "ул. Тестовая, 1",
                        "delivery": "1200", "items": [{ "product_id": product, "quantity": qty }] }),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v
    }
}

#[tokio::test]
async fn демо_каталог_по_5_карточек_на_категорию() {
    let t = setup().await;
    let (_, cats) = t.get("/api/catalog/categories", None).await;
    assert_eq!(cats.as_array().unwrap().len(), 23);
    let (_, items) = t.get("/api/catalog/products?limit=500", None).await;
    assert_eq!(items.as_array().unwrap().len(), 124);
    assert_eq!(t.num("SELECT MAX(c) FROM (SELECT COUNT(DISTINCT group_id) c FROM products GROUP BY category_id)").await, 5);
    // Опт выключен по умолчанию: заказчик хочет одну цену для всех.
    assert!(items.as_array().unwrap().iter().all(|p| p["wholesale_price"].is_null()));
}

#[tokio::test]
async fn закрытые_разделы_требуют_входа_и_роли() {
    let t = setup().await;
    assert_eq!(t.get("/api/orders", None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(t.get("/api/orders", Some("чужой-токен")).await.0, StatusCode::UNAUTHORIZED);
    let (s, _) = t.post("/api/auth/login", None, json!({ "login": "owner", "password": "неверный" })).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    let sklad = t.login("sklad").await;
    let (s, list) = t.get("/api/orders", Some(&sklad)).await;
    assert_eq!(s, StatusCode::OK);
    assert!(list[0].get("payment_status").is_none(), "кладовщик не видит оплаты");
    let (s, _) = t.post("/api/catalog/products", Some(&sklad), json!({ "name": "x" })).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(t.get("/api/clients", Some(&sklad)).await.0, StatusCode::FORBIDDEN);

    let dana = t.login("dana").await;
    assert_eq!(t.get("/api/analytics", Some(&dana)).await.0, StatusCode::FORBIDDEN);
    assert_eq!(t.get("/api/dashboard?period=month", Some(&dana)).await.0, StatusCode::OK);
    let (_, me) = t.get("/api/auth/me", Some(&dana)).await;
    assert_eq!(me["role"], "manager");
}

#[tokio::test]
async fn полный_цикл_заказа_с_нехваткой_на_сборке() {
    let t = setup().await;
    let pid = t.product().await;
    let reserved_before = t.num(&format!("SELECT reserved_milli FROM products WHERE id = '{pid}'")).await;
    let stock_before = t.num(&format!("SELECT stock_milli FROM products WHERE id = '{pid}'")).await;

    let o = t.guest_order(&pid, "4").await;
    let id = o["id"].as_str().unwrap().to_string();
    assert_eq!(o["channel"], "site");
    assert!(o["order_no"].as_str().unwrap().parse::<i64>().unwrap() > 1000);
    assert!(o["client_id"].is_string(), "заказ привязан к клиенту по телефону");
    assert_eq!(t.num(&format!("SELECT reserved_milli FROM products WHERE id = '{pid}'")).await, reserved_before + 4000);
    let item = o["items"][0]["id"].as_str().unwrap().to_string();
    let price: f64 = o["items"][0]["price"].as_str().unwrap().parse().unwrap();
    assert_eq!(o["total"].as_str().unwrap().parse::<f64>().unwrap(), price * 4.0 + 1200.0);

    let dana = t.login("dana").await;
    let sklad = t.login("sklad").await;
    // Перескочить статус нельзя.
    let (s, _) = t.patch(&format!("/api/orders/{id}"), Some(&dana), json!({ "status": "completed" })).await;
    assert_eq!(s, StatusCode::CONFLICT);
    // Подтверждает менеджер, а не кладовщик.
    let (s, _) = t.patch(&format!("/api/orders/{id}"), Some(&sklad), json!({ "status": "confirmed" })).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = t.patch(&format!("/api/orders/{id}"), Some(&dana), json!({ "status": "confirmed" })).await;
    assert_eq!(s, StatusCode::OK);

    // По учёту 4, годных 3: нехватка делает заказ проблемным.
    let (s, v) = t
        .patch(&format!("/api/orders/{id}/items/{item}/pick"), Some(&sklad), json!({ "picked": false, "short_qty": "1" }))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["is_problem"], true);
    assert!(v["problem_note"].as_str().unwrap().starts_with("Не хватает"));
    let (s, _) = t.patch(&format!("/api/orders/{id}"), Some(&sklad), json!({ "status": "assembled" })).await;
    assert_eq!(s, StatusCode::CONFLICT);

    // Менеджер правит состав прямо в карточке — проблема закрыта, резерв уменьшен.
    let (s, v) = t.patch(&format!("/api/orders/{id}/items/{item}"), Some(&dana), json!({ "quantity": "3" })).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["is_problem"], false);
    assert_eq!(t.num(&format!("SELECT reserved_milli FROM products WHERE id = '{pid}'")).await, reserved_before + 3000);
    assert_eq!(v["total"].as_str().unwrap().parse::<f64>().unwrap(), price * 3.0 + 1200.0);

    let (s, _) = t.patch(&format!("/api/orders/{id}"), Some(&sklad), json!({ "status": "assembled" })).await;
    assert_eq!(s, StatusCode::OK);
    let (s, v) = t.patch(&format!("/api/orders/{id}"), Some(&sklad), json!({ "status": "handed_over" })).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(t.num(&format!("SELECT stock_milli FROM products WHERE id = '{pid}'")).await, stock_before - 3000);
    assert_eq!(t.num(&format!("SELECT reserved_milli FROM products WHERE id = '{pid}'")).await, reserved_before);

    // После передачи отмены нет.
    let (s, _) = t.patch(&format!("/api/orders/{id}"), Some(&dana), json!({ "status": "cancelled" })).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, v) = t.patch(&format!("/api/orders/{id}"), Some(&dana), json!({ "status": "completed" })).await;
    assert_eq!(s, StatusCode::OK);
    let kinds: Vec<&str> = v["events"].as_array().unwrap().iter().map(|e| e["kind"].as_str().unwrap()).collect();
    for k in ["created", "status", "picking", "items"] {
        assert!(kinds.contains(&k), "в журнале нет {k}: {kinds:?}");
    }
}

#[tokio::test]
async fn оплата_частичная_полная_и_отмена_только_через_возврат() {
    let t = setup().await;
    let pid = t.product().await;
    let o = t.guest_order(&pid, "2").await;
    let id = o["id"].as_str().unwrap();
    let dana = t.login("dana").await;

    let (s, _) = t.post(&format!("/api/orders/{id}/payment"), Some(&dana), json!({ "payment_status": "paid", "payment_method": "online" })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "онлайн-оплаты нет");
    let (s, v) = t
        .post(&format!("/api/orders/{id}/payment"), Some(&dana), json!({ "payment_status": "partial", "payment_method": "kaspi_transfer", "paid": "1000" }))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["payment_status"], "partial");
    assert_eq!(v["paid"], "1000.00");
    let (s, _) = t.patch(&format!("/api/orders/{id}"), Some(&dana), json!({ "status": "cancelled" })).await;
    assert_eq!(s, StatusCode::CONFLICT, "оплаченный заказ не отменяется без возврата");
    let (s, v) = t.post(&format!("/api/orders/{id}/payment"), Some(&dana), json!({ "payment_status": "paid" })).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["due"], "0.00");
    let (s, _) = t.post(&format!("/api/orders/{id}/payment"), Some(&dana), json!({ "payment_status": "refunded" })).await;
    assert_eq!(s, StatusCode::OK);
    let reserved = t.num(&format!("SELECT reserved_milli FROM products WHERE id = '{pid}'")).await;
    let (s, _) = t.patch(&format!("/api/orders/{id}"), Some(&dana), json!({ "status": "cancelled", "cancelled_reason": "передумал" })).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(t.num(&format!("SELECT reserved_milli FROM products WHERE id = '{pid}'")).await, reserved - 2000);
    let (_, d) = t.get("/api/dashboard?period=today", Some(&dana)).await;
    assert_eq!(d["payments_received"], "0.00", "оплата и возврат взаимно погасились");
}

#[tokio::test]
async fn персональные_цены_только_по_входу() {
    let t = setup().await;
    let pid = t.product().await;
    let owner = t.login("owner").await;
    let cid = t.one("SELECT id FROM clients ORDER BY id LIMIT 1").await;
    let (s, _) = t.call(Method::PUT, &format!("/api/clients/{cid}/prices/{pid}"), Some(&owner), Some(json!({ "price": "99" }))).await;
    assert_eq!(s, StatusCode::OK);
    let dana = t.login("dana").await;
    let (s, _) = t.call(Method::PUT, &format!("/api/clients/{cid}/prices/{pid}"), Some(&dana), Some(json!({ "price": "1" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "персональную цену заводит владелец");

    // Гость не увидит чужую договорную цену, подставив client_id.
    let (_, p) = t.get(&format!("/api/catalog/products/{pid}?client_id={cid}"), None).await;
    assert!(p["personal_price"].is_null());
    let (_, p) = t.get(&format!("/api/catalog/products/{pid}?client_id={cid}"), Some(&dana)).await;
    assert_eq!(p["personal_price"], "99.00");

    // Клиент с персональными ценами не регистрируется сам — пароль выдаёт владелец.
    let phone = t.one(&format!("SELECT phone FROM clients WHERE id = '{cid}'")).await;
    let (s, _) = t.post("/api/auth/client/register", None, json!({ "name": "x", "phone": phone, "password": "secret1" })).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, _) = t.post(&format!("/api/clients/{cid}/password"), Some(&owner), json!({ "password": "secret1" })).await;
    assert_eq!(s, StatusCode::OK);
    let (s, v) = t.post("/api/auth/client/login", None, json!({ "phone": phone, "password": "secret1" })).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let token = v["token"].as_str().unwrap().to_string();
    let (_, p) = t.get(&format!("/api/catalog/products/{pid}"), Some(&token)).await;
    assert_eq!(p["personal_price"], "99.00");

    // Заказ вошедшего клиента считается по его цене.
    let (s, o) = t
        .post("/api/orders", Some(&token), json!({ "client_name": "Клиент", "client_phone": phone, "items": [{ "product_id": pid, "quantity": "2" }] }))
        .await;
    assert_eq!(s, StatusCode::OK, "{o}");
    assert_eq!(o["items"][0]["price_kind"], "personal");
    assert_eq!(o["total"], "198.00");
    let (_, mine) = t.get("/api/me/orders", Some(&token)).await;
    assert!(mine.as_array().unwrap().iter().any(|x| x["id"] == o["id"]));
}

#[tokio::test]
async fn регистрация_профиль_и_отслеживание() {
    let t = setup().await;
    let (s, v) = t.post("/api/auth/client/register", None, json!({ "name": "Айгерим", "phone": "8 (777) 000-11-22", "password": "123456" })).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let token = v["token"].as_str().unwrap().to_string();
    let (s, _) = t.post("/api/auth/client/register", None, json!({ "name": "Ещё", "phone": "+77770001122", "password": "123456" })).await;
    assert_eq!(s, StatusCode::CONFLICT, "второй раз тот же телефон не регистрируется");
    let (_, me) = t.get("/api/me", Some(&token)).await;
    assert_eq!(me["orders_count"], 0);
    assert_eq!(t.get("/api/orders", Some(&token)).await.0, StatusCode::FORBIDDEN);

    let pid = t.product().await;
    let o = t.guest_order(&pid, "1").await;
    let no = o["order_no"].as_str().unwrap();
    let (s, tr) = t.get(&format!("/api/orders/track?order_no={no}&phone=87015550011"), None).await;
    assert_eq!(s, StatusCode::OK, "{tr}");
    assert_eq!(tr["status_title"], "Новый");
    let (s, _) = t.get(&format!("/api/orders/track?order_no={no}&phone=87010000000"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn каталог_владельца_товар_характеристики_архив() {
    let t = setup().await;
    let owner = t.login("owner").await;
    let (s, c) = t.post("/api/catalog/categories", Some(&owner), json!({ "name": "Пищевые красители" })).await;
    assert_eq!(s, StatusCode::OK, "{c}");
    assert_eq!(c["slug"], "pischevye-krasiteli");
    let (s, p) = t
        .post(
            "/api/catalog/products",
            Some(&owner),
            json!({ "category_id": c["id"], "name": "Краситель гелевый", "sku": "TEST-GEL-1", "price": "1500",
                    "stock": "12", "storage_location": "Склад А / стеллаж 9 / полка 1",
                    "attributes": [{ "name": "Цвет", "value": "Красный" }, { "name": "Объём", "value": "10 мл" }] }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert_eq!(p["attributes"].as_array().unwrap().len(), 2);
    assert_eq!(p["available_qty"], "12.000");
    let pid = p["id"].as_str().unwrap();
    let (s, _) = t.post("/api/catalog/products", Some(&owner), json!({ "category_id": c["id"], "name": "Дубль", "sku": "TEST-GEL-1", "price": "1" })).await;
    assert_eq!(s, StatusCode::CONFLICT, "артикул уникален");
    let (s, p2) = t.call(Method::PUT, &format!("/api/catalog/products/{pid}/attributes"), Some(&owner), Some(json!([{ "name": "Тип", "value": "Гелевый" }]))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(p2["attributes"][0]["name"], "Тип");

    // Товар с заказом не удаляется — только снимается с продажи.
    t.guest_order(pid, "1").await;
    let (s, _) = t.call(Method::DELETE, &format!("/api/catalog/products/{pid}"), Some(&owner), None).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, _) = t.post(&format!("/api/catalog/products/{pid}/archive"), Some(&owner), json!({})).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(t.get(&format!("/api/catalog/products/{pid}"), None).await.0, StatusCode::NOT_FOUND);
    let (s, _) = t.call(Method::DELETE, &format!("/api/catalog/categories/{}", c["id"].as_str().unwrap()), Some(&owner), None).await;
    assert_eq!(s, StatusCode::CONFLICT, "непустую категорию не удалить");
}

#[tokio::test]
async fn накладная_дашборд_аналитика_склад() {
    let t = setup().await;
    let pid = t.product().await;
    let o = t.guest_order(&pid, "1").await;
    let owner = t.login("owner").await;
    let (s, html) = t.get(&format!("/api/orders/{}/invoice", o["id"].as_str().unwrap()), Some(&owner)).await;
    assert_eq!(s, StatusCode::OK);
    assert!(html.as_str().unwrap().contains("НАКЛАДНАЯ НА ОТПУСК ЗАПАСОВ"));

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/analytics/visit")
        .header("content-type", "application/json")
        .header("user-agent", "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) Mobile/15E148 Safari/604.1")
        .body(Body::from(json!({ "path": "/catalog", "session_id": "s1" }).to_string()))
        .unwrap();
    assert_eq!(t.app.clone().oneshot(req).await.unwrap().status(), StatusCode::OK);
    let (s, a) = t.get("/api/analytics?period=today", Some(&owner)).await;
    assert_eq!(s, StatusCode::OK, "{a}");
    assert_eq!(a["by_device"][0]["key"], "mobile");
    assert_eq!(a["visitors"], 1);

    let (_, d) = t.get("/api/dashboard?period=today", Some(&owner)).await;
    assert_eq!(d["orders"]["count"], 1);
    assert!(d["low_stock_count"].as_i64().unwrap() > 0);

    let sklad = t.login("sklad").await;
    let (_, low) = t.get("/api/warehouse/stock?status=low", Some(&sklad)).await;
    assert!(low.as_array().unwrap().iter().all(|x| x["status"] == "low"));
    let (s, v) = t.post("/api/warehouse/movements", Some(&sklad), json!({ "product_id": pid, "movement_type": "adjustment", "quantity": "-1" })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "корректировка без причины: {v}");
    let (s, v) = t.post("/api/warehouse/movements", Some(&sklad), json!({ "product_id": pid, "movement_type": "in", "quantity": "10", "reason": "накладная 15" })).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["employee_name"], "Кладовщик");
    let (s, q) = t.get("/api/warehouse/assembly", Some(&sklad)).await;
    assert_eq!(s, StatusCode::OK);
    assert!(q.as_array().unwrap().iter().all(|x| x.get("payment_status").is_none()));
}

#[tokio::test]
async fn сотрудники_и_последний_владелец() {
    let t = setup().await;
    let owner = t.login("owner").await;
    let (s, e) = t.post("/api/employees", Some(&owner), json!({ "name": "Менеджер Асель", "login": "asel", "role": "manager", "password": "asel123" })).await;
    assert_eq!(s, StatusCode::OK, "{e}");
    let (s, _) = t.post("/api/auth/login", None, json!({ "login": "asel", "password": "asel123" })).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = t.patch("/api/employees/e-owner", Some(&owner), json!({ "role": "manager" })).await;
    assert_eq!(s, StatusCode::CONFLICT, "без владельца система остаться не должна");
}

// ─── Исправления по ревью PR #13 и задачам #35–#37 ──────────────────────────

#[tokio::test]
async fn после_пяти_неверных_паролей_пауза() {
    let t = setup().await;
    for _ in 0..5 {
        let (s, _) = t.post("/api/auth/login", None, json!({ "login": "aliya", "password": "неверный" })).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
    }
    let (s, v) = t.post("/api/auth/login", None, json!({ "login": "aliya", "password": PW })).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS, "в паузе не пускает и с верным паролем: {v}");
    // Другой логин не страдает.
    assert!(!t.login("dana").await.is_empty());
}

#[tokio::test]
async fn правка_оплаченного_заказа_пересчитывает_оплату() {
    let t = setup().await;
    let pid = t.product().await;
    let o = t.guest_order(&pid, "2").await;
    let (id, item) = (o["id"].as_str().unwrap(), o["items"][0]["id"].as_str().unwrap());
    let dana = t.login("dana").await;
    let (s, _) = t.post(&format!("/api/orders/{id}/payment"), Some(&dana), json!({ "payment_status": "paid", "payment_method": "cash" })).await;
    assert_eq!(s, StatusCode::OK);

    // Больше товара — долг: «частично», а не «оплачен».
    let (s, v) = t.patch(&format!("/api/orders/{id}/items/{item}"), Some(&dana), json!({ "quantity": "3" })).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["payment_status"], "partial");
    assert_ne!(v["due"], "0.00");

    // Меньше товара — переплата видна.
    let (_, v) = t.patch(&format!("/api/orders/{id}/items/{item}"), Some(&dana), json!({ "quantity": "1" })).await;
    assert_eq!(v["payment_status"], "paid");
    assert!(v["overpaid"].is_string(), "переплата в ответе: {v}");
    assert!(v["events"].as_array().unwrap().iter().any(|e| e["message"].as_str().unwrap().starts_with("Переплата")));
}

#[tokio::test]
async fn доставку_с_сайта_считает_сервер() {
    let t = setup().await;
    let pid = t.product().await;
    let (s, o) = t
        .post("/api/orders", None, json!({ "client_name": "Гость", "client_phone": "+77015550000", "delivery_method": "courier",
            "city": "Алматы", "delivery_address": "ул. Абая, 1", "delivery": "0", "items": [{ "product_id": pid, "quantity": "1" }] }))
        .await;
    assert_eq!(s, StatusCode::OK, "{o}");
    assert_eq!(o["delivery"], "2500.00", "гость не может прислать доставку 0");
    let (_, o) = t
        .post("/api/orders", None, json!({ "client_name": "Гость", "client_phone": "+77015550000", "delivery_method": "pickup",
            "items": [{ "product_id": pid, "quantity": "1" }] }))
        .await;
    assert_eq!(o["delivery"], "0.00");
}

#[tokio::test]
async fn сборка_отдаёт_счётчики_не_перекрывая_итог() {
    let t = setup().await;
    let sklad = t.login("sklad").await;
    let (_, q) = t.get("/api/warehouse/assembly", Some(&sklad)).await;
    let first = &q.as_array().unwrap()[0];
    assert!(first["total"].is_string(), "total — деньги, строкой: {first}");
    assert!(first["items_count"].is_number() && first["picked_count"].is_number());
}

#[tokio::test]
async fn поиск_без_учёта_регистра_по_словам_и_категории() {
    let t = setup().await;
    let (_, big) = t.get("/api/catalog/products?search=Шоколад&limit=500", None).await;
    let (_, small) = t.get("/api/catalog/products?search=шоколад&limit=500", None).await;
    assert!(!big.as_array().unwrap().is_empty());
    assert_eq!(big.as_array().unwrap().len(), small.as_array().unwrap().len(), "регистр не важен");
    let (_, words) = t.get("/api/catalog/products?search=коробка%20торт&limit=500", None).await;
    assert!(!words.as_array().unwrap().is_empty(), "слова не обязаны идти подряд");
    let cat = t.one("SELECT id FROM categories WHERE name LIKE 'Коробки для торта%' LIMIT 1").await;
    let in_cat = t.num(&format!("SELECT COUNT(*) FROM products WHERE category_id = '{cat}'")).await;
    let (_, torte) = t.get("/api/catalog/products?search=торт&limit=500", None).await;
    assert!(torte.as_array().unwrap().len() as i64 >= in_cat, "название категории участвует в поиске");
    let sklad = t.login("sklad").await;
    let (_, st) = t.get("/api/warehouse/stock?search=склад%20а", Some(&sklad)).await;
    assert!(!st.as_array().unwrap().is_empty(), "поиск по полке на складе");
}

#[tokio::test]
async fn у_отменённого_после_возврата_нет_долга() {
    let t = setup().await;
    let pid = t.product().await;
    let o = t.guest_order(&pid, "1").await;
    let id = o["id"].as_str().unwrap();
    let dana = t.login("dana").await;
    t.post(&format!("/api/orders/{id}/payment"), Some(&dana), json!({ "payment_status": "paid", "payment_method": "cash" })).await;
    t.post(&format!("/api/orders/{id}/payment"), Some(&dana), json!({ "payment_status": "refunded" })).await;
    let (s, v) = t.patch(&format!("/api/orders/{id}"), Some(&dana), json!({ "status": "cancelled" })).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["due"], "0.00");
}

#[tokio::test]
async fn фото_без_ключей_отвечает_503_остальное_работает() {
    // В этом процессе ключей Pexels и Cloudinary нет — как на сервере до настройки.
    let t = setup().await;
    let owner = t.login("owner").await;
    let id = t.product().await;

    let (s, v) = t.get("/api/catalog/photo-suggestions?q=box", Some(&owner)).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{v}");
    assert!(v["error"].as_str().unwrap().contains("PEXELS_API_KEY"));

    let (s, v) = t.post(&format!("/api/catalog/products/{id}/photo"), Some(&owner), json!({ "pexels_id": 1 })).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{v}");

    let (s, _) = t.get("/api/catalog/photo-suggestions?q=box", None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let sklad = t.login("sklad").await;
    let (s, _) = t.get("/api/catalog/photo-suggestions?q=box", Some(&sklad)).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    let (s, _) = t.get(&format!("/api/catalog/products/{id}"), None).await;
    assert_eq!(s, StatusCode::OK, "каталог работает без ключей");
}
