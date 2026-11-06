# Неделя 12 — Swap через Raydium

**Задание:** обменять токены из vault через CLMM-пул Raydium с ограничением на минимальную сумму получения (slippage).

Продолжение сквозного проекта Liquidity Vault:
[8 — каркас](https://github.com/zandarbek402-dotcom/Week-8) →
[9 — депозит](https://github.com/zandarbek402-dotcom/Week9) →
[10 — CPI и вывод](https://github.com/zandarbek402-dotcom/week10) →
[11 — Raydium CLMM-пул в devnet](https://github.com/zandarbek402-dotcom/Week11) →
**12 — swap из vault через Raydium**.

## Что сделано

Новая инструкция **`swap_via_raydium(amount_in, min_amount_out, sqrt_price_limit_x64)`**: vault обменивает **свои** токены в CLMM-пуле Raydium (W11A/W11B из недели 11) через **CPI в `swap_v2`**, подписанный **PDA vault**.

```
 admin/keeper ──swap_via_raydium(amount_in, min_amount_out)──► Liquidity Vault program
                                                                 │
   1. проверки: admin, не paused, pool == config.pool, аккаунты пула совпадают с PoolState
   2. slippage-защита #1: min_amount_out ≥ spot_out × (1 − max_slippage_bps)
   3. CPI ──► Raydium CLMM swap_v2   (подпись: config PDA ["config", mint, bump])
   │            vault W11A ──► pool vault A        pool vault B ──► vault W11B
   │            Raydium сам проверяет other_amount_threshold = min_amount_out
   4. slippage-защита #2: реальное изменение балансов: received ≥ min_amount_out, spent ≤ amount_in
   5. emit!(VaultSwapEvent)
```

### Две независимые защиты от проскальзывания

| # | Где | Что проверяется | Защищает от |
|---|---|---|---|
| 1 | наша программа, **до** обмена | `min_amount_out` не ниже `spot_out × (1 − max_slippage_bps)`, где `spot_out` — выход по текущей цене пула за вычетом комиссии | `min_amount_out = 1` (сэндвич-атака: бот забирает всё), устаревшая котировка |
| 2 | Raydium (`other_amount_threshold`) | фактический выход ≥ `min_amount_out` с учётом price impact | движение цены / большой обмен |
| 2′ | наша программа, **после** CPI | `reload()` токен-аккаунтов: получено ≥ `min_amount_out`, потрачено ≤ `amount_in` | ошибка или подмена внешней программы — не доверяем CPI вслепую |

`max_slippage_bps` задаёт admin в `set_pool` (по умолчанию 1%, максимум 10%).

### Новые инструкции

| Инструкция | Кто | Что делает |
|---|---|---|
| `set_pool(max_slippage_bps)` | admin | привязывает CLMM-пул Raydium (owner = программа Raydium, один из mint = underlying) |
| `init_strategy_account()` | admin | создаёт ATA vault-PDA для второго токена пула (W11B) |
| `swap_via_raydium(amount_in, min_amount_out, sqrt_price_limit_x64)` | admin/keeper | обмен через Raydium CLMM; `remaining_accounts` = bitmap extension + tick arrays |

### Интеграция с Raydium без крейта Raydium — [`raydium.rs`](programs/liquidity_vault/src/raydium.rs)

- `swap_v2` собирается вручную: дискриминатор `sha256("global:swap_v2")[..8]`, аргументы Borsh, 13 аккаунтов **в порядке Raydium** (сверено с `swapV2Instruction` в Raydium SDK) + remaining accounts. Вызов через `invoke_signed` с seeds vault-PDA.
- `PoolState` и `AmmConfig` читаются по байтовым смещениям (zero-copy). Смещения **проверены на живом пуле в devnet**:

| Поле PoolState | offset | значение в нашем пуле |
|---|---|---|
| amm_config | 9 | `CD4aJt…` |
| token_mint_0 / 1 | 73 / 105 | W11A / W11B |
| token_vault_0 / 1 | 137 / 169 | `2yA2g5…` / `58oRgZ…` |
| observation_key | 201 | `CLAch3…` |
| liquidity (u128) | 237 | 16189909849 |
| sqrt_price_x64 (u128) | 253 | 26031301204159531733 |
| tick_current (i32) | 269 | 6888 |
| AmmConfig.trade_fee_rate (u32) | 47 | 2500 (= 0.25%) |

- Цена из `sqrt_price_x64` (Q64.64): `out = in × (s/2^64)^2` (A→B) или `in / (s/2^64)^2` (B→A) — целочисленно, без переполнения (`mul_shr64`).
- Программа Raydium: devnet `DRayAUgENGQBKVaX8owNhgzkEDyoHTGVEGHVJT1E9pfH` по умолчанию, mainnet `CAMMCzo5…` через `--features mainnet`.

### Проверки аккаунтов

- `pool_state`: `address = config.pool` и `owner = Raydium CLMM`.
- `amm_config`, `observation_state`, `input_vault`/`output_vault` **сверяются с PoolState** (нельзя подсунуть чужой vault пула).
- Направление обмена (`zero_for_one`) определяется по mint-ам; mint-ы должны совпадать с пулом.
- Токен-аккаунты vault: `token::authority = config` — платит и получает только сам vault.
- `clmm_program`, `token_program_2022`, `memo_program` проверяются по адресу.

## Тесты

### Интеграционные: настоящий Raydium + настоящий пул из devnet — [`tests-rs/tests/vault_swap.rs`](tests-rs/tests/vault_swap.rs)

`solana-program-test` (in-process банк Solana) загружает:
- нашу программу `fixtures/liquidity_vault.so` (собрана `cargo build-sbf`);
- **байткод Raydium CLMM**, выгруженный из devnet (`solana program dump`);
- **реальное состояние пула W11A/W11B** из недели 11: PoolState, AmmConfig, vaults, observation, bitmap, tick arrays (`solana account --output json`).

**Результат: 6 / 6 тестов прошли** ([полный лог](tests-rs/test-output.txt)):

| Тест | Что проверяет | Результат |
|---|---|---|
| `vault_swaps_through_raydium_with_slippage_protection` | vault продаёт 20 W11A → W11B через Raydium, затем меняет половину W11B обратно | ✅ см. ниже |
| `rejects_min_amount_out_below_slippage_floor` | `min_amount_out = 1` и «на 1 меньше порога» | ✅ `MinOutTooLow` (6015) — наша программа |
| `raydium_enforces_min_amount_out_on_price_impact` | 200 W11A с `min_amount_out` = спот (без учёта price impact) | ✅ `TooLittleOutputReceived` — **сработала проверка Raydium**, транзакция полностью откатилась |
| `only_admin_can_swap` | чужой подписант | ✅ `Unauthorized` (6007) |
| `rejects_wrong_pool_vaults` | перепутаны vault A/B пула | ✅ `PoolAccountMismatch` (6013) |
| `paused_vault_cannot_swap` | vault на паузе | ✅ `VaultPaused` (6001) |

Обмен из vault через **настоящий байткод Raydium CLMM**:

```
pool tick 6888, spot out for 20 W11A = 39.727863 W11B, min_amount_out = 39.330584 (−1%)
  Program 264Fck…  invoke [1]                ← Liquidity Vault
  Program DRayAU…  invoke [2]                ← CPI в Raydium CLMM swap_v2 (подпись vault PDA)
  Program log: Vault swap: 20000000 in -> 39658900 out (min 39330584, spot 39727863), tick 6888 -> 6853
vault: W11A 500 -> 480, W11B 0 -> 39.6589
обратный обмен: W11A 489.958727, W11B 19.82945
```

Получено 39.6589 W11B: это ≥ min (39.3306) и < spot (39.7279) на величину price impact ≈ 0.17%. Цена пула сдвинулась: tick 6888 → 6853.

### Найденный и исправленный баг (с недели 8)

При запуске на настоящем SBF-рантайме `initialize_vault` падал с `Access violation in unknown section at address 0x0`. Причина: `cargo build-sbf` сообщал `try_accounts … Stack offset of 4376 exceeded max offset of 4096`. Кадр стека превышал лимит SBF (4 КБ) и затирал ссылки на аккаунты. Хостовые `cargo test` такое не ловят. Исправлено через `Box<Account<…>>` в [`initialize_vault.rs`](programs/liquidity_vault/src/instructions/initialize_vault.rs); теперь сборка проходит без предупреждений о стеке.

### Unit-тесты (`cargo test --lib`)

18 тестов: математика shares/fees (недели 9–10) + `spot_amount_out` на реальной цене пула, `price = 1`, дискриминатор `swap_v2`.

## Структура

```
programs/liquidity_vault/src/
├── raydium.rs                      # NEW: id, PoolState view, spot price, swap_v2 CPI
├── instructions/
│   ├── swap_via_raydium.rs         # NEW
│   ├── set_pool.rs                 # NEW
│   ├── init_strategy_account.rs    # NEW
│   └── … (deposit, withdraw, … из недель 8–10)
├── state.rs                        # + max_slippage_bps
├── events.rs                       # + VaultSwapEvent
└── errors.rs                       # + InvalidPool, PoolAccountMismatch, MinOutTooLow, …
tests-rs/                           # NEW: интеграционные тесты против Raydium
idl/liquidity_vault.json            # IDL (11 инструкций)
scripts/vault-swap.ts               # клиент для devnet: котировка Raydium SDK → swap_via_raydium
scripts/merge-idl.js                # сборка IDL без Anchor CLI
```

## Запуск

```bash
# unit-тесты
cargo test --lib

# сборка программы (Solana CLI 1.18; Cargo.lock закреплён под platform-tools rustc 1.75)
cargo build-sbf --manifest-path programs/liquidity_vault/Cargo.toml

# интеграционные тесты против Raydium
cp target/deploy/liquidity_vault.so tests-rs/fixtures/
cd tests-rs && cargo test -- --nocapture
```

Swap в devnet (нужно ~3.5 SOL на деплой программы):

```bash
solana program deploy target/deploy/liquidity_vault.so --program-id <keypair> --url devnet
npm install
npx tsx scripts/vault-swap.ts 20 50     # 20 W11A, slippage 0.5%
```

## Ограничения и дальше

- Пока `total_assets` vault считает только свободный underlying (W11A). Если vault держит W11B после обмена, цена share временно занижена. Оценка второго токена и CLMM-позиции в TVL — следующий шаг (неделя 13+: vault открывает позицию ликвидности через CPI `open_position` / `increase_liquidity`).
- Защита #1 использует спотовую цену самого пула, которую можно сдвинуть внутри той же транзакции. В продакшене нужен внешний оракул (Pyth) или TWAP из `ObservationState`.
