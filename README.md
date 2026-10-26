# AXMVA

Axiom, версия Anthropic. Рабочее имя: **axmv2**.

Проект когнитивной архитектуры, преемник старых линий Axiom/NeuroGraph. Единая абстракция для любого элемента схемы — **passport**: идентичность, контракт, связи и то, что видит модель (см. [specs/Passport.md](specs/Passport.md)).

## Статус

`hypothesis` — проект в фазе кристаллизации структуры: материал изучается, инварианты и спецификация passport фиксируются, реализация кода пока не начата (см. [ROADMAP.md](ROADMAP.md)). Этот репозиторий — базовый каркас, без кода.

## Документация

Этот репозиторий — единственное рабочее пространство проекта. Начни с [00-Index.md](00-Index.md):

- [Invariants.md](Invariants.md) — нейминг и неизменяемые правила.
- [Deferred.md](Deferred.md) — отложенные вопросы.
- [specs/Passport.md](specs/Passport.md) — спецификация passport.
- [guides/Passport-guide.md](guides/Passport-guide.md) — как написать passport для нового элемента.

Материалы-источники (идеи из старых линий Axiom/NeuroGraph) остаются во внешнем архиве `AXMV2/` (Obsidian vault) и сюда не копируются — документы отсюда ссылаются на них, но этот репозиторий ими не управляет.

Корневой `passport.yaml` в этом репозитории — passport самой сборки (`kind: assembly`), формат описан в `specs/Passport.md`.

## Лицензия

[GNU AGPLv3](LICENSE).

## Участие

См. [CONTRIBUTING.md](CONTRIBUTING.md).
