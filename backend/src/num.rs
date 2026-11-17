//! Перевод денег и количеств между целым хранением и строкой наружу.

pub const TIYN: i64 = 100;
pub const MILLI: i64 = 1000;

pub fn tiyn_to_string(v: i64) -> String {
    format!("{}.{:02}", v / TIYN, (v % TIYN).abs())
}

pub fn milli_to_string(v: i64) -> String {
    format!("{}.{:03}", v / MILLI, (v % MILLI).abs())
}

/// Разбирает «2», «2.5», «2.500» в целое с заданным числом знаков.
/// Через строку, а не через f64: 0.1 + 0.2 в float даёт 0.30000000000000004,
/// и на суммах заказа это вылезет расхождением в копейках.
pub fn parse_scaled(s: &str, decimals: u32) -> Result<i64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("пустое значение".into());
    }
    let (int_part, frac_part) = s.split_once('.').unwrap_or((s, ""));

    if frac_part.len() > decimals as usize {
        return Err(format!("не больше {decimals} знаков после точки"));
    }
    if !int_part.chars().all(|c| c.is_ascii_digit()) || int_part.is_empty() {
        return Err(format!("«{s}» не число"));
    }
    if !frac_part.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("«{s}» не число"));
    }

    let scale = 10i64.pow(decimals);
    let int: i64 = int_part.parse().map_err(|_| format!("«{s}» слишком большое"))?;

    let mut frac = frac_part.to_string();
    while frac.len() < decimals as usize {
        frac.push('0');
    }
    let frac: i64 = if frac.is_empty() { 0 } else { frac.parse().unwrap_or(0) };

    int.checked_mul(scale)
        .and_then(|v| v.checked_add(frac))
        .ok_or_else(|| format!("«{s}» слишком большое"))
}

/// Понадобится, когда появится создание и правка товаров через API.
#[allow(dead_code)]
pub fn parse_tiyn(s: &str) -> Result<i64, String> {
    parse_scaled(s, 2)
}

pub fn parse_milli(s: &str) -> Result<i64, String> {
    parse_scaled(s, 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn форматирование() {
        assert_eq!(tiyn_to_string(188000), "1880.00");
        assert_eq!(tiyn_to_string(0), "0.00");
        assert_eq!(tiyn_to_string(5), "0.05");
        assert_eq!(milli_to_string(2500), "2.500");
        assert_eq!(milli_to_string(24000), "24.000");
    }

    #[test]
    fn разбор() {
        assert_eq!(parse_tiyn("1880").unwrap(), 188000);
        assert_eq!(parse_tiyn("1880.50").unwrap(), 188050);
        assert_eq!(parse_milli("2.5").unwrap(), 2500);
        assert_eq!(parse_milli("24").unwrap(), 24000);
    }

    #[test]
    fn мусор_отвергается() {
        assert!(parse_milli("").is_err());
        assert!(parse_milli("abc").is_err());
        assert!(parse_milli("-5").is_err());
        assert!(parse_tiyn("1.555").is_err());
    }

    #[test]
    fn сумма_заказа_без_потери_точности() {
        // 3 × 0.1 кг по 19.99 ₸ — на float дало бы расхождение
        let qty = parse_milli("0.1").unwrap();
        let price = parse_tiyn("19.99").unwrap();
        let sum: i64 = (0..3).map(|_| price * qty / MILLI).sum();
        assert_eq!(sum, 597); // 5.97 ₸
    }
}
