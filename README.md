# Team 1
Учебная практика «Команда разработки».
Заказчик: . Срок: 28.11.26.

## Состав команды
| Участник | Роль |
|----------|------|
| Михель Павел | Тимлид / Руководитель проекта (PM); Backend-разработчик |
| Фалюш Данил | Frontend-разработчик; UI/UX-дизайнер |
| Пономарев Роман | UI/UX-дизайнер; Frontend-разработчик |
| Токубаев Омар | QA-инженер (тестировщик); Бизнес-аналитик / Технический писатель |
| Одинцов Константин | Бизнес-аналитик / Технический писатель; QA-инженер (тестировщик) |

## Запуск проекта
Нужны Node.js 20+ и backend из ветки `feat/backend-api` (Rust или Docker). Два терминала:

```bash
cd backend && cp .env.example .env && cargo run   # API: http://localhost:8080
cd frontend && npm start                          # сайт: http://localhost:5174, CRM: http://localhost:5174/crm/
```

Демо-входы в CRM — `owner`, `dana`, `sklad`, пароль из `BOOTSTRAP_PASSWORD` (в образце `bakeloom-demo`).
Подробности — в [frontend/README.md](frontend/README.md) и `backend/README.md`.

## Документация
(ссылки на документ требований, API, стек)
