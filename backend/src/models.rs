//! Строки БД (`*Row`) и то, что уходит наружу (`*View`).
//!
//! Наружу деньги и количества отдаются строкой: «1880.00», «2.500».
//! Строка, а не число, чтобы JSON-парсер на фронте не превратил их в float
//! и не вернул 1879.9999999998.

use serde::{Deserialize, Serialize};

use crate::num::{milli_to_string, tiyn_to_string};

// ─── Категории ───────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
pub struct Category {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub parent_id: Option<String>,
    pub sort_order: i64,
    pub created_at: String,
    /// Сколько товаров в продаже: пустые категории сайт может скрыть.
    pub products_count: i64,
}

#[derive(Debug, Deserialize)]
pub struct CategoryInput {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub parent_id: Option<String>,
    pub sort_order: Option<i64>,
}

// ─── Товары ──────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ProductRow {
    pub id: String,
    pub category_id: String,
    pub name: String,
    pub sku: Option<String>,
    pub description: Option<String>,
    pub price_tiyn: i64,
    pub unit: String,
    pub is_weight: i64,
    pub stock_milli: i64,
    pub reserved_milli: i64,
    pub min_stock_milli: Option<i64>,
    pub min_order_milli: i64,
    pub order_step_milli: i64,
    pub image_url: Option<String>,
    pub is_archived: i64,
    /// Варианты одного товара (100 г / 500 г / 1 кг) делят общий group_id
    /// и показываются одной карточкой с переключателем.
    pub group_id: Option<String>,
    pub variant_name: Option<String>,
    pub wholesale_price_tiyn: Option<i64>,
    pub wholesale_min_qty: Option<i64>,
    pub supply_pack_size: Option<String>,
    pub storage_location: Option<String>,
    pub image_name: Option<String>,
}

pub const PRODUCT_COLS: &str = "id, category_id, name, sku, description, price_tiyn, unit,
     is_weight, stock_milli, reserved_milli, min_stock_milli, min_order_milli, order_step_milli,
     image_url, is_archived, group_id, variant_name, wholesale_price_tiyn,
     wholesale_min_qty, supply_pack_size, storage_location, image_name";

/// Характеристики — произвольные пары, не фиксированные колонки: единого
/// набора для всех товаров нет, владелец заводит новые сам (ФТ-Т-03).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Attribute {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Deserialize)]
pub struct AttributeRow {
    pub product_id: String,
    pub name: String,
    pub value: String,
}

#[derive(Debug, Serialize)]
pub struct ProductView {
    pub id: String,
    pub category_id: String,
    pub name: String,
    pub sku: Option<String>,
    pub description: Option<String>,
    /// Цена для всех. Заказчик хочет одну цену для всех (В3 27:47).
    pub price: String,
    pub unit: String,
    pub is_weight: bool,
    pub stock_qty: String,
    pub available_qty: String,
    /// in_stock | low | out — для бейджа «в наличии / мало / нет».
    pub stock_status: &'static str,
    /// Минимальное количество в заказе и шаг изменения (ФТ-Т-05).
    pub min_order: String,
    pub order_step: String,
    pub image_url: Option<String>,
    pub is_archived: bool,
    pub group_id: Option<String>,
    pub variant_name: Option<String>,
    /// Опт по умолчанию выключен (WHOLESALE_ENABLED), тогда здесь null.
    pub wholesale_price: Option<String>,
    pub wholesale_min_qty: Option<i64>,
    pub storage_location: Option<String>,
    /// Имя будущего файла фотографии из таблицы заказчика.
    pub image_name: Option<String>,
    /// Цена вошедшего клиента, если для него заведена. Показывать вместо `price`.
    pub personal_price: Option<String>,
    pub attributes: Vec<Attribute>,
}

pub fn stock_status(available: i64, min_stock: Option<i64>) -> &'static str {
    if available <= 0 {
        "out"
    } else if min_stock.is_some_and(|m| available < m) {
        "low"
    } else {
        "in_stock"
    }
}

impl ProductView {
    pub fn from_row(r: ProductRow, attributes: Vec<Attribute>) -> Self {
        let available = (r.stock_milli - r.reserved_milli).max(0);
        let wholesale = crate::config::get().wholesale_enabled;
        Self {
            id: r.id,
            category_id: r.category_id,
            name: r.name,
            sku: r.sku,
            description: r.description,
            price: tiyn_to_string(r.price_tiyn),
            unit: r.unit,
            is_weight: r.is_weight != 0,
            stock_qty: milli_to_string(r.stock_milli),
            available_qty: milli_to_string(available),
            stock_status: stock_status(available, r.min_stock_milli),
            min_order: milli_to_string(r.min_order_milli),
            order_step: milli_to_string(r.order_step_milli),
            image_url: r.image_url,
            is_archived: r.is_archived != 0,
            group_id: r.group_id,
            variant_name: r.variant_name,
            wholesale_price: r.wholesale_price_tiyn.filter(|_| wholesale).map(tiyn_to_string),
            wholesale_min_qty: r.wholesale_min_qty.filter(|_| wholesale),
            storage_location: r.storage_location,
            image_name: r.image_name,
            personal_price: None,
            attributes,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ProductQuery {
    pub category_id: Option<String>,
    pub group_id: Option<String>,
    /// Только для сотрудника: посмотреть каталог глазами клиента.
    /// Покупатель видит свои цены по входу, параметр ему не нужен.
    pub client_id: Option<String>,
    pub search: Option<String>,
    pub in_stock: Option<bool>,
    /// Только для владельца: показать и снятые с продажи.
    pub include_archived: Option<bool>,
    /// name | price_asc | price_desc | new
    pub sort: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ProductInput {
    pub category_id: Option<String>,
    pub name: Option<String>,
    pub sku: Option<String>,
    pub description: Option<String>,
    pub price: Option<String>,
    pub unit: Option<String>,
    pub is_weight: Option<bool>,
    pub min_order: Option<String>,
    pub order_step: Option<String>,
    pub min_stock: Option<String>,
    /// Карточка, к которой относится вариант. Без него товар — своя карточка.
    pub group_id: Option<String>,
    pub variant_name: Option<String>,
    pub wholesale_price: Option<String>,
    pub wholesale_min_qty: Option<i64>,
    pub storage_location: Option<String>,
    pub image_url: Option<String>,
    pub image_name: Option<String>,
    pub attributes: Option<Vec<Attribute>>,
    /// Начальный остаток при создании — оформляется приходом на склад.
    pub stock: Option<String>,
}

// ─── Заказы ──────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct OrderRow {
    pub id: String,
    pub order_no: Option<String>,
    pub client_id: Option<String>,
    pub client_name: String,
    pub client_phone: Option<String>,
    pub status: String,
    pub items_tiyn: i64,
    pub delivery_tiyn: i64,
    pub total_tiyn: i64,
    pub paid_tiyn: i64,
    pub channel: String,
    pub city: Option<String>,
    pub delivery_method: String,
    pub delivery_address: Option<String>,
    pub payment_method: String,
    pub payment_status: String,
    pub is_urgent: i64,
    pub is_problem: i64,
    pub problem_note: Option<String>,
    pub note: Option<String>,
    pub source: Option<String>,
    pub assigned_to: Option<String>,
    pub assigned_name: Option<String>,
    pub cancelled_reason: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Колонки заказа с именем ответственного менеджера (ФТ-М-08).
pub const ORDER_COLS: &str = "o.id, o.order_no, o.client_id, o.client_name, o.client_phone,
     o.status, o.items_tiyn, o.delivery_tiyn, o.total_tiyn, o.paid_tiyn, o.channel, o.city,
     o.delivery_method, o.delivery_address, o.payment_method, o.payment_status,
     o.is_urgent, o.is_problem, o.problem_note, o.note, o.source, o.assigned_to,
     e.name AS assigned_name, o.cancelled_reason, o.created_at, o.updated_at";

pub const ORDER_FROM: &str = "orders o LEFT JOIN employees e ON e.id = o.assigned_to";

#[derive(Debug, Serialize)]
pub struct OrderView {
    pub id: String,
    pub order_no: Option<String>,
    pub client_id: Option<String>,
    pub client_name: String,
    pub client_phone: Option<String>,
    pub status: String,
    pub items_total: String,
    pub delivery: String,
    pub total: String,
    /// Оплату отмечает менеджер: QR статический, система о платеже не узнаёт.
    /// Кладовщику поля оплаты не отдаются (ФТ-Р-02).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    /// Переплата после правки состава или доставки — вернуть разницу клиенту.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overpaid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_method: Option<String>,
    /// unpaid | partial | paid | refunded
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_status: Option<String>,
    pub channel: String,
    pub city: Option<String>,
    pub delivery_method: String,
    pub delivery_address: Option<String>,
    pub is_urgent: bool,
    /// Сбой на этапе, например нехватка при сборке (ФТ-З-08).
    pub is_problem: bool,
    pub problem_note: Option<String>,
    pub note: Option<String>,
    pub source: Option<String>,
    pub assigned_to: Option<String>,
    pub assigned_name: Option<String>,
    pub cancelled_reason: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<OrderRow> for OrderView {
    fn from(r: OrderRow) -> Self {
        // У отменённого заказа и после возврата долга нет.
        let closed = r.status == "cancelled" || r.payment_status == "refunded";
        let due = if closed { 0 } else { (r.total_tiyn - r.paid_tiyn).max(0) };
        let overpaid = (r.paid_tiyn > r.total_tiyn && r.payment_status != "refunded")
            .then(|| tiyn_to_string(r.paid_tiyn - r.total_tiyn));
        Self {
            id: r.id,
            order_no: r.order_no,
            client_id: r.client_id,
            client_name: r.client_name,
            client_phone: r.client_phone,
            status: r.status,
            items_total: tiyn_to_string(r.items_tiyn),
            delivery: tiyn_to_string(r.delivery_tiyn),
            total: tiyn_to_string(r.total_tiyn),
            paid: Some(tiyn_to_string(r.paid_tiyn)),
            due: Some(tiyn_to_string(due)),
            overpaid,
            payment_method: Some(r.payment_method),
            payment_status: Some(r.payment_status),
            channel: r.channel,
            city: r.city,
            delivery_method: r.delivery_method,
            delivery_address: r.delivery_address,
            is_urgent: r.is_urgent != 0,
            is_problem: r.is_problem != 0,
            problem_note: r.problem_note,
            note: r.note,
            source: r.source,
            assigned_to: r.assigned_to,
            assigned_name: r.assigned_name,
            cancelled_reason: r.cancelled_reason,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

impl OrderView {
    /// Кладовщик не видит оплаты (ФТ-Р-02).
    pub fn without_payment(mut self) -> Self {
        self.paid = None;
        self.due = None;
        self.overpaid = None;
        self.payment_method = None;
        self.payment_status = None;
        self
    }
}

#[derive(Debug, Deserialize)]
pub struct OrderItemRow {
    pub id: String,
    pub product_id: Option<String>,
    pub product_name: String,
    pub product_sku: Option<String>,
    pub unit: String,
    pub price_tiyn: i64,
    pub quantity_milli: i64,
    pub discount_percent: i64,
    pub subtotal_tiyn: i64,
    pub price_kind: Option<String>,
    pub picked: i64,
    pub short_milli: i64,
    pub storage_location: Option<String>,
    pub available_milli: Option<i64>,
}

pub const ITEM_COLS: &str = "i.id, i.product_id, i.product_name, i.product_sku, i.unit,
     i.price_tiyn, i.quantity_milli, i.discount_percent, i.subtotal_tiyn, i.price_kind,
     i.picked, i.short_milli, p.storage_location,
     p.stock_milli - p.reserved_milli AS available_milli";

pub const ITEM_FROM: &str = "order_items i LEFT JOIN products p ON p.id = i.product_id";

#[derive(Debug, Serialize)]
pub struct OrderItemView {
    pub id: String,
    pub product_id: Option<String>,
    pub product_name: String,
    pub product_sku: Option<String>,
    pub unit: String,
    pub price: String,
    pub quantity: String,
    pub discount_percent: i64,
    /// retail | wholesale | personal — по какой цене посчитана строка.
    pub price_kind: Option<String>,
    pub subtotal: String,
    /// Отметки кладовщика при сборке.
    pub picked: bool,
    pub short_qty: String,
    /// Где лежит и сколько свободно сейчас — для карточки заказа (ФТ-М-05).
    pub storage_location: Option<String>,
    pub available_qty: Option<String>,
}

impl From<OrderItemRow> for OrderItemView {
    fn from(r: OrderItemRow) -> Self {
        Self {
            id: r.id,
            product_id: r.product_id,
            product_name: r.product_name,
            product_sku: r.product_sku,
            unit: r.unit,
            price: tiyn_to_string(r.price_tiyn),
            quantity: milli_to_string(r.quantity_milli),
            discount_percent: r.discount_percent,
            price_kind: r.price_kind,
            subtotal: tiyn_to_string(r.subtotal_tiyn),
            picked: r.picked != 0,
            short_qty: milli_to_string(r.short_milli),
            storage_location: r.storage_location,
            available_qty: r.available_milli.map(|v| milli_to_string(v.max(0))),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct EventRow {
    pub id: String,
    pub kind: String,
    pub message: String,
    pub amount_tiyn: Option<i64>,
    pub actor_kind: String,
    pub actor_name: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct EventView {
    pub id: String,
    /// created | status | payment | items | picking | problem | urgent |
    /// delivery | assigned | comment
    pub kind: String,
    pub message: String,
    pub amount: Option<String>,
    pub actor_kind: String,
    pub actor_name: Option<String>,
    pub created_at: String,
}

impl From<EventRow> for EventView {
    fn from(r: EventRow) -> Self {
        Self {
            id: r.id,
            kind: r.kind,
            message: r.message,
            amount: r.amount_tiyn.map(tiyn_to_string),
            actor_kind: r.actor_kind,
            actor_name: r.actor_name,
            created_at: r.created_at,
        }
    }
}

/// Прошлый заказ клиента — для истории в карточке заказа.
#[derive(Debug, Deserialize, Serialize)]
pub struct OrderBrief {
    pub id: String,
    pub order_no: Option<String>,
    pub status: String,
    pub total: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct OrderDetail {
    #[serde(flatten)]
    pub order: OrderView,
    pub items: Vec<OrderItemView>,
    /// Журнал действий — для сотрудников.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub events: Option<Vec<EventView>>,
    /// Другие заказы этого клиента, новые сверху (ФТ-М-05).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_orders: Option<Vec<OrderBrief>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateOrderItem {
    pub product_id: String,
    /// Строкой: «2» или «2.5». Числом пришлёт float и потеряет точность.
    pub quantity: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateOrder {
    pub client_name: String,
    pub client_phone: Option<String>,
    /// Только для сотрудника, который заводит заказ за клиента.
    /// Покупатель привязывается по входу.
    pub client_id: Option<String>,
    /// Канал: site | site_legacy | whatsapp | instagram | phone | other.
    /// С сайта всегда site, другие каналы заводит менеджер (ФТ-М-04).
    pub source: Option<String>,
    pub note: Option<String>,
    pub city: Option<String>,
    /// pickup | courier | cdek | kazpost. По умолчанию самовывоз.
    pub delivery_method: Option<String>,
    /// Для доставки обязателен (ФТ-О-02).
    pub delivery_address: Option<String>,
    /// Стоимость доставки строкой: «1200.00».
    pub delivery: Option<String>,
    /// none | cash | kaspi_transfer. Онлайн-оплаты нет (ФТ-П-01).
    pub payment_method: Option<String>,
    /// Только для сотрудника.
    pub is_urgent: Option<bool>,
    pub assigned_to: Option<String>,
    pub items: Vec<CreateOrderItem>,
}

#[derive(Debug, Deserialize)]
pub struct ConfirmPayment {
    /// unpaid | partial | paid | refunded
    pub payment_status: String,
    /// cash | kaspi_transfer. Для partial и paid обязателен.
    pub payment_method: Option<String>,
    /// Сколько получено. Для paid по умолчанию весь итог,
    /// для partial обязателен и меньше итога.
    pub paid: Option<String>,
}

/// Все поля необязательные: меняется то, что передано.
#[derive(Debug, Deserialize)]
pub struct UpdateOrder {
    pub status: Option<String>,
    pub cancelled_reason: Option<String>,
    pub assigned_to: Option<String>,
    pub is_urgent: Option<bool>,
    pub is_problem: Option<bool>,
    pub problem_note: Option<String>,
    pub note: Option<String>,
    pub city: Option<String>,
    pub delivery_method: Option<String>,
    pub delivery_address: Option<String>,
    pub delivery: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OrderQuery {
    pub status: Option<String>,
    pub payment_status: Option<String>,
    pub channel: Option<String>,
    pub assigned_to: Option<String>,
    pub client_id: Option<String>,
    pub urgent: Option<bool>,
    pub problem: Option<bool>,
    /// Номер заказа, имя клиента или телефон.
    pub search: Option<String>,
    /// Даты в формате 2026-11-15, по UTC.
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

// ─── Склад ───────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct StockRow {
    pub id: String,
    pub name: String,
    pub sku: Option<String>,
    pub variant_name: Option<String>,
    pub category_id: String,
    pub unit: String,
    pub stock_milli: i64,
    pub reserved_milli: i64,
    pub min_stock_milli: Option<i64>,
    pub storage_location: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct StockView {
    pub product_id: String,
    pub product_name: String,
    pub sku: Option<String>,
    pub variant_name: Option<String>,
    pub category_id: String,
    pub unit: String,
    pub stock_qty: String,
    pub reserved_qty: String,
    pub available_qty: String,
    pub min_stock_qty: Option<String>,
    pub low_stock: bool,
    /// in_stock | low | out
    pub status: &'static str,
    /// Где лежит на складе. Сканера нет, ищут по полке (ФТ-СК-02).
    pub storage_location: Option<String>,
    pub updated_at: String,
}

impl From<StockRow> for StockView {
    fn from(r: StockRow) -> Self {
        let available = (r.stock_milli - r.reserved_milli).max(0);
        Self {
            product_id: r.id,
            product_name: r.name,
            sku: r.sku,
            variant_name: r.variant_name,
            category_id: r.category_id,
            unit: r.unit,
            stock_qty: milli_to_string(r.stock_milli),
            reserved_qty: milli_to_string(r.reserved_milli),
            available_qty: milli_to_string(available),
            min_stock_qty: r.min_stock_milli.map(milli_to_string),
            low_stock: r.min_stock_milli.is_some_and(|m| available < m),
            status: stock_status(available, r.min_stock_milli),
            storage_location: r.storage_location,
            updated_at: r.updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct MovementRow {
    pub id: String,
    pub product_id: String,
    pub product_name: Option<String>,
    pub movement_type: String,
    pub quantity_milli: i64,
    pub reason: Option<String>,
    pub order_id: Option<String>,
    pub employee_id: Option<String>,
    pub employee_name: Option<String>,
    pub storage_location: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct MovementView {
    pub id: String,
    pub product_id: String,
    pub product_name: Option<String>,
    pub movement_type: String,
    pub quantity: String,
    pub reason: Option<String>,
    pub order_id: Option<String>,
    pub employee_id: Option<String>,
    pub employee_name: Option<String>,
    pub storage_location: Option<String>,
    pub created_at: String,
}

impl From<MovementRow> for MovementView {
    fn from(r: MovementRow) -> Self {
        Self {
            id: r.id,
            product_id: r.product_id,
            product_name: r.product_name,
            movement_type: r.movement_type,
            quantity: milli_to_string(r.quantity_milli),
            reason: r.reason,
            order_id: r.order_id,
            employee_id: r.employee_id,
            employee_name: r.employee_name,
            storage_location: r.storage_location,
            created_at: r.created_at,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateMovement {
    pub product_id: String,
    /// in — приход, out — расход, adjustment — корректировка
    pub movement_type: String,
    /// Строкой. Для adjustment допускается «-» в начале.
    pub quantity: String,
    /// Основание: накладная, просрочка, пересчёт. Для корректировки обязательно.
    pub reason: Option<String>,
    /// Куда положили при приёмке (ФТ-СК-05)
    pub storage_location: Option<String>,
}
