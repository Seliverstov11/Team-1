// Контакты магазина — одно место на весь сайт: подвал, страница «Контакты», самовывоз, отслеживание заказа.
// Пустое значение (null) — на сайте «уточняется», кнопки звонка и WhatsApp не показываются.
// Те же реквизиты для накладной — в переменных сервера SHOP_NAME / SHOP_ADDRESS / SHOP_BIN.

export const SHOP = {
  name: 'BAKELOOM',
  address: 'Абылай хана, 50',       // SHOP_ADDRESS от заказчика, 18.11
  city: null,                        // город самовывоза — уточнить у заказчика
  phone: null,                       // '+7 (7XX) XXX-XX-XX' — уточнить у заказчика
  whatsapp: null,                    // номер для wa.me, если отличается от телефона (ФТ-С-02)
  email: null,
  hours: 'Ежедневно 9:00 — 21:00',   // уточнить у заказчика
  instagram: null,                   // 'bakeloom.kz' без @
  chatWhatsappUrl: 'https://chat.whatsapp.com/BlkPTekTfsXGVRMwxixAoP',
}

const digits = s => String(s ?? '').replace(/\D/g, '')

export const shopInfo = {
  ...SHOP,
  get fullAddress() { return [SHOP.city, SHOP.address].filter(Boolean).join(', ') || 'Адрес уточняется' },
  get phoneText() { return SHOP.phone ?? 'Телефон уточняется' },
  get tel() { return SHOP.phone ? `tel:+${digits(SHOP.phone)}` : null },
  get wa() { const n = digits(SHOP.whatsapp ?? SHOP.phone); return n ? `https://wa.me/${n}` : null },
  get chat() { return SHOP.chatWhatsappUrl },
  get mailto() { return SHOP.email ? `mailto:${SHOP.email}` : null },
  get map() { return `https://2gis.kz/search/${encodeURIComponent([SHOP.city ?? 'Казахстан', SHOP.address].join(', '))}` },
  get complete() { return Boolean(SHOP.phone && SHOP.city) },
}

// Что сейчас с заказом — человеческими словами, для покупателя (отслеживание, профиль, экран «Заказ принят»)
export function orderNextStep(o) {
  const pickup = o?.delivery_method === 'pickup'
  switch (o?.status) {
    case 'new': return { text: 'Заказ принят. Менеджер позвонит, подтвердит состав и доставку', tone: 'info' }
    case 'confirmed': return { text: 'Заказ подтверждён — собираем его на складе. Когда будет готов, статус сменится на «Собран»', tone: 'info' }
    case 'assembled': return pickup
      ? { text: `Заказ готов — можно забирать! Самовывоз: ${shopInfo.fullAddress}, ${SHOP.hours.toLowerCase()}`, tone: 'ready' }
      : { text: 'Заказ собран и готов к отправке — скоро передадим в доставку', tone: 'ready' }
    case 'handed_over': return pickup
      ? { text: 'Заказ выдан. Спасибо за покупку!', tone: 'done' }
      : { text: 'Заказ передан в доставку', tone: 'info' }
    case 'shipped': return { text: 'Заказ в пути', tone: 'info' }
    case 'delivered': return { text: 'Заказ доставлен', tone: 'done' }
    case 'completed': return { text: 'Заказ завершён. Спасибо за покупку!', tone: 'done' }
    case 'cancelled': return { text: `Заказ отменён${o.cancelled_reason ? `: ${o.cancelled_reason}` : ''}`, tone: 'cancelled' }
    default: return { text: '', tone: 'info' }
  }
}
