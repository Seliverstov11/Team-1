// Справочники заказа — по таблице заказчика и контракту backend/openapi.yaml.

// Восемь статусов из таблицы заказчика. Списание со склада — на handed_over:
// по условиям магазина товар уходит при выдаче или передаче в доставку.
export const STATUSES = {
  new:         { label: 'Новый',           client: 'Принят',             badge: 'bg-status-new-soft text-status-new' },
  confirmed:   { label: 'Подтверждён',     client: 'Подтверждён',        badge: 'bg-status-confirmed-soft text-status-confirmed' },
  assembled:   { label: 'Собран',          client: 'Собран',             badge: 'bg-brand-dark-soft text-brand-dark' },
  handed_over: { label: 'Выдан / передан', client: 'Выдан или передан в доставку', badge: 'bg-status-shipping-soft text-status-shipping' },
  shipped:     { label: 'Отправлен',       client: 'В пути',             badge: 'bg-brand-gold-soft text-ink' },
  delivered:   { label: 'Доставлен',       client: 'Доставлен',          badge: 'bg-success-soft text-success' },
  completed:   { label: 'Завершён',        client: 'Завершён',           badge: 'bg-brand-dark text-brand-light' },
  cancelled:   { label: 'Отменён',         client: 'Отменён',            badge: 'bg-error-soft text-error' },
}
export const STATUS_ORDER = ['new', 'confirmed', 'assembled', 'handed_over', 'shipped', 'delivered', 'completed', 'cancelled']
// Основной путь — для степпера (cancelled в стороне)
export const FLOW = STATUS_ORDER.slice(0, 7)

// Те же переходы проверяет backend (409 на остальные)
export const NEXT = {
  new: ['confirmed', 'cancelled'],
  confirmed: ['assembled', 'new', 'cancelled'],
  assembled: ['handed_over', 'confirmed', 'cancelled'],
  handed_over: ['shipped', 'delivered', 'completed'],
  shipped: ['delivered'],
  delivered: ['completed'],
  completed: [],
  cancelled: [],
}
export const STATUS_HINT = {
  new: 'Вернуть в новые — с клиентом ещё не договорились',
  confirmed: 'Состав, сумма и способ получения согласованы с клиентом',
  assembled: 'Склад собрал заказ, товар ещё в резерве',
  handed_over: 'Выдан клиенту или передан в доставку — остаток списывается со склада',
  shipped: 'Отправлен курьером, СДЭК или Казпочтой',
  delivered: 'Клиент получил заказ',
  completed: 'Заказ закрыт',
  cancelled: 'Резерв снимается. Оплаченный заказ отменяется только с возвратом',
}
// Заказ в работе: ещё не выдан и не отменён (под него стоит резерв)
export const ACTIVE = ['new', 'confirmed', 'assembled']
export const OPEN = ['new', 'confirmed', 'assembled', 'handed_over', 'shipped', 'delivered']

export const CHANNELS = {
  site: 'Сайт',
  site_legacy: 'Старая страница',
  whatsapp: 'WhatsApp',
  instagram: 'Instagram',
  phone: 'Телефон',
  other: 'Другое',
}

export const DELIVERY = {
  pickup: 'Самовывоз',
  courier: 'Курьер',
  cdek: 'СДЭК',
  kazpost: 'Казпочта',
}

// Онлайн-оплаты нет: QR Kaspi статический, наличные. online — только чтение старых записей
export const PAYMENT_METHODS = {
  none: 'Не указан',
  cash: 'Наличные',
  kaspi_transfer: 'Kaspi (QR / перевод)',
  online: 'Онлайн',
}

export const PAYMENT = {
  unpaid:   { label: 'Не оплачен', badge: 'bg-error-soft text-error' },
  partial:  { label: 'Частично',   badge: 'bg-status-confirmed-soft text-status-confirmed' },
  paid:     { label: 'Оплачен',    badge: 'bg-success-soft text-success' },
  refunded: { label: 'Возврат',    badge: 'bg-line text-ink-muted' },
}
// unpaid с ненулевой суммой — частичная предоплата
export const paymentKey = o => (o.payment_status === 'unpaid' && o.paid && !/^0+(\.0+)?$/.test(o.paid) ? 'partial' : o.payment_status)
