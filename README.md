# Team 1
Учебная практика «Команда разработки».
Проект: интернет-магазин и CRM BAKELOOM. Срок: 28.11.26.

## Состав команды
| Участник | Роль |
|----------|------|
| Михель Павел | Тимлид / Руководитель проекта (PM); Backend-разработчик |
| Фалюш Данил | Frontend-разработчик; UI/UX-дизайнер |
| Пономарев Роман | UI/UX-дизайнер; Frontend-разработчик |
| Токубаев Омар | QA-инженер (тестировщик); Бизнес-аналитик / Технический писатель |
| Одинцов Константин | Бизнес-аналитик / Технический писатель; QA-инженер (тестировщик) |

## Запуск проекта

Нужны Docker или Rust для backend и Node.js 20+ для сайта и CRM. Два терминала:

```bash
# 1. Backend — API для сайта и CRM
cd backend && cp .env.example .env && docker compose up --build   # или cargo run; API: http://localhost:8080

# 2. Сайт и CRM
cd frontend && npm start          # сайт: http://localhost:5174, CRM: http://localhost:5174/crm/
```

Демо-входы в CRM — `owner`, `dana`, `aliya`, `sklad`, пароль из `BOOTSTRAP_PASSWORD` (в образце `bakeloom-demo`).
Подробности — [`backend/README.md`](backend/README.md) и [`frontend/README.md`](frontend/README.md).

## Документация

- Контракт API — [`backend/openapi.yaml`](backend/openapi.yaml)
- Backend: запуск, роли, жизненный цикл заказа — [`backend/README.md`](backend/README.md)
- Сайт и CRM: запуск, входы, CRM как приложение — [`frontend/README.md`](frontend/README.md)
- Тест-план — [`docs/test-plan.md`](docs/test-plan.md)
- Регламент команды — [`docs/Регламент_команды_и_роли.pdf`](docs/Регламент_команды_и_роли.pdf)
