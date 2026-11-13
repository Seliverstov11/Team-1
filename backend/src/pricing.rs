//! Какая цена попадает в строку заказа.
//!
//! Цен у товара три: розница, опт «от N единиц» и персональная цена
//! конкретного клиента. Порядок такой:
//!
//! 1. Персональная цена, если она заведена, — это цена, о которой с клиентом
//!    договорились отдельно (28:03: «клиент входит под своей учётной записью
//!    и видит цены, отличные от общих»).
//! 2. Иначе опт, если количество дотянуло до `wholesale_min_qty`.
//! 3. Иначе розница.
//!
//! Что делать, когда персональная цена оказалась выше оптовой, заказчик не
//! говорил. Сейчас персональная всё равно выигрывает: она договорная, и
//! менять её молча на другую — худшее из двух. Вопрос вынесен заказчику.

use crate::num::MILLI;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriceKind {
    Retail,
    Wholesale,
    Personal,
}

impl PriceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            PriceKind::Retail => "retail",
            PriceKind::Wholesale => "wholesale",
            PriceKind::Personal => "personal",
        }
    }
}

/// `quantity_milli` — количество ×1000, `wholesale_min_qty` — в единицах.
pub fn effective_price(
    retail_tiyn: i64,
    wholesale_tiyn: Option<i64>,
    wholesale_min_qty: Option<i64>,
    personal_tiyn: Option<i64>,
    quantity_milli: i64,
) -> (i64, PriceKind) {
    if let Some(p) = personal_tiyn {
        return (p, PriceKind::Personal);
    }

    if let (Some(w), Some(min_qty)) = (wholesale_tiyn, wholesale_min_qty)
        && min_qty > 0
        && quantity_milli >= min_qty * MILLI
    {
        return (w, PriceKind::Wholesale);
    }

    (retail_tiyn, PriceKind::Retail)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RETAIL: i64 = 520000; // 5 200 ₸
    const WHOLESALE: i64 = 450000; // 4 500 ₸

    #[test]
    fn розница_пока_количество_не_дотянуло() {
        let (p, k) = effective_price(RETAIL, Some(WHOLESALE), Some(10), None, 9 * MILLI);
        assert_eq!((p, k), (RETAIL, PriceKind::Retail));
    }

    #[test]
    fn опт_ровно_на_границе() {
        let (p, k) = effective_price(RETAIL, Some(WHOLESALE), Some(10), None, 10 * MILLI);
        assert_eq!((p, k), (WHOLESALE, PriceKind::Wholesale));
    }

    #[test]
    fn без_оптовой_цены_всегда_розница() {
        let (p, k) = effective_price(RETAIL, None, Some(10), None, 100 * MILLI);
        assert_eq!((p, k), (RETAIL, PriceKind::Retail));
    }

    #[test]
    fn персональная_бьёт_и_опт_и_розницу() {
        let (p, k) = effective_price(RETAIL, Some(WHOLESALE), Some(10), Some(400000), 50 * MILLI);
        assert_eq!((p, k), (400000, PriceKind::Personal));
        let (p, k) = effective_price(RETAIL, Some(WHOLESALE), Some(10), Some(400000), 1 * MILLI);
        assert_eq!((p, k), (400000, PriceKind::Personal));
    }

    #[test]
    fn нулевой_порог_опта_не_включает_опт() {
        // Иначе опт сработал бы на нулевом количестве и на любой покупке.
        let (p, k) = effective_price(RETAIL, Some(WHOLESALE), Some(0), None, 1 * MILLI);
        assert_eq!((p, k), (RETAIL, PriceKind::Retail));
    }

    #[test]
    fn дробное_количество_сравнивается_честно() {
        // 9,999 единицы — ещё не десять.
        let (_, k) = effective_price(RETAIL, Some(WHOLESALE), Some(10), None, 9_999);
        assert_eq!(k, PriceKind::Retail);
    }
}
