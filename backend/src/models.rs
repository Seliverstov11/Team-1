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
    pub storage_location: Option<String>,
    pub image_name: Option<String>,
}

/// Характеристики — произвольные пары, не фиксированные колонки:
/// заказчик будет заводить новые сам при добавлении товара.
#[derive(Debug, Deserialize, Serialize)]
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
    pub price: String,
    pub unit: String,
    pub is_weight: bool,
    pub stock_qty: String,
    pub available_qty: String,
    /// Минимальное количество в заказе и шаг изменения
    pub min_order: String,
    pub order_step: String,
    pub image_url: Option<String>,
    pub is_archived: bool,
    pub group_id: Option<String>,
    pub variant_name: Option<String>,
    /// Оптовая цена и количество, с которого она начинает действовать.
    pub wholesale_price: Option<String>,
    pub wholesale_min_qty: Option<i64>,
    pub storage_location: Option<String>,
    /// Имя будущего файла фотографии из таблицы заказчика: сами фото
    /// подбирает команда, в каталоге их пока нет.
    pub image_name: Option<String>,
    /// Цена этого клиента, если запрос пришёл с client_id и цена заведена.
    /// Показывать её вместо розничной.
    pub personal_price: Option<String>,
    pub attributes: Vec<Attribute>,
}

impl ProductView {
    pub fn from_row(r: ProductRow, attributes: Vec<Attribute>) -> Self {
        let available = (r.stock_milli - r.reserved_milli).max(0);
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
            min_order: milli_to_string(r.min_order_milli),
            order_step: milli_to_string(r.order_step_milli),
            image_url: r.image_url,
            is_archived: r.is_archived != 0,
            group_id: r.group_id,
            variant_name: r.variant_name,
            wholesale_price: r.wholesale_price_tiyn.map(tiyn_to_string),
            wholesale_min_qty: r.wholesale_min_qty,
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
    /// Клиент, под которым смотрят каталог: для него подставляется
    /// персональная цена, если она заведена.
    pub client_id: Option<String>,
    pub search: Option<String>,
    pub in_stock: Option<bool>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
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
    pub note: Option<String>,
    pub source: Option<String>,
    pub assigned_to: Option<String>,
    pub cancelled_reason: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

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
    /// Оплату подтверждает менеджер: QR у заказчика статический,
    /// система о поступлении денег не узнаёт (26:01).
    pub paid: String,
    pub channel: String,
    pub city: Option<String>,
    pub delivery_method: String,
    pub delivery_address: Option<String>,
    pub payment_method: String,
    pub payment_status: String,
    pub note: Option<String>,
    pub source: Option<String>,
    pub assigned_to: Option<String>,
    pub cancelled_reason: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<OrderRow> for OrderView {
    fn from(r: OrderRow) -> Self {
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
            paid: tiyn_to_string(r.paid_tiyn),
            channel: r.channel,
            city: r.city,
            delivery_method: r.delivery_method,
            delivery_address: r.delivery_address,
            payment_method: r.payment_method,
            payment_status: r.payment_status,
            note: r.note,
            source: r.source,
            assigned_to: r.assigned_to,
            cancelled_reason: r.cancelled_reason,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
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
}

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
            price_kind: None,
            subtotal: tiyn_to_string(r.subtotal_tiyn),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct OrderDetail {
    #[serde(flatten)]
    pub order: OrderView,
    pub items: Vec<OrderItemView>,
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
    /// Если клиент известен, к нему применяются его персональные цены.
    pub client_id: Option<String>,
    pub source: Option<String>,
    pub note: Option<String>,
    pub city: Option<String>,
    /// pickup | courier | cdek | kazpost. По умолчанию самовывоз.
    pub delivery_method: Option<String>,
    /// Адрес или пункт выдачи. Для доставки обязателен: на оформлении
    /// заказчик просил имя, телефон и адрес (19:19).
    pub delivery_address: Option<String>,
    /// Стоимость доставки строкой: «1200.00». Считает менеджер или тариф,
    /// бэкенд её не выдумывает — служб доставки заказчик ещё не назвал.
    pub delivery: Option<String>,
    /// none | cash | kaspi_transfer. Онлайн-оплаты не будет (26:01).
    pub payment_method: Option<String>,
    pub items: Vec<CreateOrderItem>,
}

/// Оплату подтверждает человек: QR у заказчика статический, система
/// о поступлении денег не узнаёт.
#[derive(Debug, Deserialize)]
pub struct ConfirmPayment {
    /// unpaid | paid | refunded
    pub payment_status: String,
    pub payment_method: Option<String>,
    /// Сколько получено, строкой. По умолчанию — весь итог заказа.
    pub paid: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateOrderStatus {
    pub status: String,
    pub cancelled_reason: Option<String>,
    pub assigned_to: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OrderQuery {
    pub status: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

// ─── Склад ───────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct StockRow {
    pub id: String,
    pub name: String,
    pub sku: Option<String>,
    pub unit: String,
    pub stock_milli: i64,
    pub reserved_milli: i64,
    pub min_stock_milli: Option<i64>,
    pub storage_location: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StockView {
    pub product_id: String,
    pub product_name: String,
    pub sku: Option<String>,
    pub unit: String,
    pub stock_qty: String,
    pub reserved_qty: String,
    pub available_qty: String,
    pub min_stock_qty: Option<String>,
    pub low_stock: bool,
    /// Где лежит на складе. Сканера у заказчика нет, ищут по полке.
    pub storage_location: Option<String>,
}

impl From<StockRow> for StockView {
    fn from(r: StockRow) -> Self {
        let available = (r.stock_milli - r.reserved_milli).max(0);
        Self {
            product_id: r.id,
            product_name: r.name,
            sku: r.sku,
            unit: r.unit,
            stock_qty: milli_to_string(r.stock_milli),
            reserved_qty: milli_to_string(r.reserved_milli),
            available_qty: milli_to_string(available),
            min_stock_qty: r.min_stock_milli.map(milli_to_string),
            low_stock: r.min_stock_milli.is_some_and(|m| available < m),
            storage_location: r.storage_location,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct MovementRow {
    pub id: String,
    pub product_id: String,
    pub movement_type: String,
    pub quantity_milli: i64,
    pub reason: Option<String>,
    pub order_id: Option<String>,
    pub employee_id: Option<String>,
    pub storage_location: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct MovementView {
    pub id: String,
    pub product_id: String,
    pub movement_type: String,
    pub quantity: String,
    pub reason: Option<String>,
    pub order_id: Option<String>,
    pub employee_id: Option<String>,
    pub storage_location: Option<String>,
    pub created_at: String,
}

impl From<MovementRow> for MovementView {
    fn from(r: MovementRow) -> Self {
        Self {
            id: r.id,
            product_id: r.product_id,
            movement_type: r.movement_type,
            quantity: milli_to_string(r.quantity_milli),
            reason: r.reason,
            order_id: r.order_id,
            employee_id: r.employee_id,
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
    /// Строкой. Для out и adjustment допускается «-» в начале.
    pub quantity: String,
    pub reason: Option<String>,
    pub employee_id: Option<String>,
    /// Куда положили при приёмке
    pub storage_location: Option<String>,
}
