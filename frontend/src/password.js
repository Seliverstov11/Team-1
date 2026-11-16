// Требования к паролю — общие для покупателей и сотрудников.
// Сервер требует от 6 символов; здесь строже: так решили на макетах входа (13.11).

const RULES = [
  [p => p.length >= 8, 'от 8 символов'],
  [p => /[a-zа-яё]/.test(p), 'строчная буква'],
  [p => /[A-ZА-ЯЁ]/.test(p), 'заглавная буква'],
  [p => /\d/.test(p), 'цифра'],
  [p => /[^\p{L}\d]/u.test(p), 'спецсимвол'],
]

// Чего не хватает: ['цифра', 'спецсимвол']
export const passwordProblems = p => RULES.filter(([ok]) => !ok(p ?? '')).map(([, label]) => label)

// 0–4 для индикатора и подпись
export function passwordStrength(p) {
  if (!p) return { score: 0, label: '' }
  const passed = RULES.length - passwordProblems(p).length
  const score = Math.min(4, Math.max(1, passed - 1 + (p.length >= 12 ? 1 : 0)))
  return { score, label: ['', 'слабый', 'средний', 'хороший', 'надёжный'][score] }
}
