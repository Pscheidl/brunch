# Backend contract

Same-origin JSON API under `/api`. Amounts are integer Czech haléře (CZK). Product IDs are positive int64, user/order IDs UUID strings. Errors `{error:string}`. Except the RSA-verified Teya webhook, all POST/PUT/PATCH requests use `Content-Type: application/json` plus `X-Brunch-Request: 1`. Cookies HttpOnly, SameSite=Lax, Secure in production. No permissive CORS.

## Authentication

- POST `/api/auth/request` `{email}` always generic `{message}`. Email link opens `/?login_token=...`. New accounts created only after redemption.
- POST `/api/auth/verify` `{token}` -> `{user:{id,email,is_admin}}`, sets session cookie. One use, 15 minute validity.
- GET `/api/auth/me` -> `{user:{id,email,is_admin}}` or 401.
- POST `/api/auth/logout` `{}` -> `{ok:true}` clears session.

## Shop

- GET `/api/products` -> array of products `{id,name,description,category,tag,weight,price_cents,vegan,ingredients,allergens,image,active,stock,version}`. `image` relative asset URL or empty. `stock` nullable int (null unlimited). Only active products.
- GET `/api/settings` -> `{delivery_fee_cents,free_delivery_from_cents,delivery_postal_codes:[string],online_payment_enabled:bool}` threshold nullable. Postal codes are sorted canonical five-digit strings for active delivery areas. An empty list blocks new orders.
- GET `/api/delivery-options` -> `{dates:[{value:"YYYY-MM-DD",label:string}],slots:[string]}` three next working days calculated in Europe/Prague, excluding weekends and Czech public holidays. Slots are `07:00–09:00`, `09:00–11:00`, `11:00–13:00`. These are requested delivery windows, capacity booking is out of scope.
- POST `/api/orders` -> `{id,number,status,payment_method,subtotal_cents,delivery_fee_cents,total_cents,...}` with body `{idempotency_key:uuid,email,name?,phone?,organization?,street,city,postal_code,notes?,delivery_date:"YYYY-MM-DD",delivery_slot,payment_method:"card"|"card_on_delivery"|"cash_on_delivery",items:[{product_id,quantity}],expected_total_cents}`. Email/street/city/postal code required. Reject outdated expected total with 409. Prices and delivery recalculated server-side. Snapshots saved transactionally with stock. Delivery orders start `new` and queue business/customer confirmations. Online orders start `awaiting_payment`, queue a customer pending notice and a persistent payment attempt in the same transaction. Business fulfilment notification waits for verified payment. Same idempotency key and payload replays existing order, altered payload 409. Account attachment only for authenticated matching verified email. No association of previous guest orders by guessed email.
- Checkout validates five-digit Czech postal codes and membership in active delivery areas, 1–50 distinct products, 1–99 units per product, available requested delivery date/slot, and total up to 1,000,000,000 haléře. All Unicode whitespace and U+FEFF are removed from postal codes before validation, storage and idempotency comparison. Other characters are rejected. Inactive or unconfigured postal codes return 400 without creating an order, reserving stock or queuing email. Successful idempotent replays remain available even if the area has since been deactivated. Optional name and organization are at most 200 characters, phone 40, street 250, city 100 and notes 2,000. New accepted orders are limited to 20 per email and 500 globally per hour with HTTP 429. Idempotent replays do not consume these limits.
- GET `/api/orders/mine` -> array of own orders (authenticated).

## Images

- POST `/api/admin/images` (admin only) JSON `{filename,content_base64}` -> `{url:"/api/images/{uuid}"}`. Source JPEG/PNG/WebP up to 2 MiB, max4096px per side and16 megapixels,3MiB JSON body limit. Server validates, applies EXIF orientation and re-encodes without metadata, stores immutable image bytes in PostgreSQL.
- GET `/api/images/{uuid}` publicly returns sanitized JPEG/PNG, immutable one-year cache and ETag support.

## Admin (session is_admin required for every endpoint)

- GET `/api/admin/products` -> array incl inactive.
- POST `/api/admin/products` product fields (id/version omitted) -> product
- PUT `/api/admin/products/{id}` product fields incl `version` -> product, optimistic conflict 409
- GET `/api/admin/products/{id}/prices` -> array `{id,price_cents,created_at,changed_by}`
- GET `/api/admin/orders?status=&q=&delivery_date=YYYY-MM-DD&page=1` -> `{orders:[...],total,page}` 50 per page. Optional `delivery_date` selects the stored delivery day, independent of creation time, and combines with status/search. Empty or omitted date includes every day, invalid dates or noncanonical formats return 400. Count and rows use the same filters, with stable `created_at DESC,id DESC` ordering. orders contain snapshot `items:[{product_id,name,quantity,unit_price_cents,total_cents}]`, delivery/contact data, `version`, `payment_status` and timestamps.
- GET `/api/admin/orders/{id}` -> order detail incl items and `history:[{status,payment_status,created_at,changed_by}]`
- PATCH `/api/admin/orders/{id}` `{status:"awaiting_payment"|"new"|"confirmed"|"preparing"|"out_for_delivery"|"delivered"|"cancelled",payment_status:"unpaid"|"paid",version}` -> order. Sequential flow is `new → confirmed → preparing → out_for_delivery → delivered`. Online payment can never be manually marked paid and an unpaid online order cannot advance. Only verified terminal failed/expired sessions allow unpaid online cancellation. Delivery payment can be marked paid only during `out_for_delivery` or `delivered`, and `delivered` requires `paid`. Stock restored once on cancellation. Delivered/cancelled are terminal. Cancellation is available before delivery only while unpaid. Paid cannot revert to unpaid, and paid cancellation requires future refund handling. Each actual status change also queues a customer email transactionally.
- GET `/api/admin/settings` -> `{delivery_fee_cents,free_delivery_from_cents,order_email,version}`
- PUT `/api/admin/settings` same fields -> settings (version conflict 409)
- GET `/api/admin/delivery-areas` -> array `{id,postal_code,label,active,version}`, including inactive areas.
- POST `/api/admin/delivery-areas` `{postal_code,label?,active?}` -> area. Label defaults to empty and is at most 100 characters, active defaults to true. Canonical postal codes are unique even across inactive areas. Duplicates return 409.
- PUT `/api/admin/delivery-areas/{id}` `{postal_code,label?,active?,version}` -> area, stale version or duplicate code returns 409. Deactivate with `active:false`, reactivate with `active:true`. Updates serialize against checkout through the shared shop settings row so an accepted order uses a consistent delivery configuration.
- GET `/api/admin/customers?page=1` -> `{customers:[{id,email,is_admin,created_at,order_count}],total,page}`
- GET `/api/admin/mail` -> array `{id,to_email,subject,status,attempts,last_error,created_at,sent_at}` latest 100, status pending/sent/failed
- POST `/api/admin/mail/{id}/retry` `{}` -> `{ok:true}` requeues failed email

## Shared server interfaces

`AppState { pool: PgPool, config: Arc<Config> }` in crate root, `ApiError` implements IntoResponse with constructors `bad_request`, `unauthorized`, `forbidden`, `conflict`, `not_found`, From sqlx::Error. `type ApiResult<T> = Result<T,ApiError>`.
`auth::current_user(&AppState,&HeaderMap) -> ApiResult<Option<User>>`, `auth::require_user(...) -> ApiResult<User>`, `auth::require_admin(...) -> ApiResult<User>`. User `{id:Uuid,email:String,is_admin:bool}`.
`mail::enqueue(&mut PgConnection,to:&str,subject:&str,body:&str) -> ApiResult<()>` to insert outbox within transaction. Config fields `public_url`, `order_email` etc.
`commerce::routes() -> Router<AppState>` includes full `/api/...` routes. `commerce::seed(&PgPool,&str)` inserts default settings and six products only when empty, no production data overrides.

## Teya Hosted Checkout

- POST `/api/orders/{id}/payment` body `{idempotency_key:uuid}` returns `{status,payment_status,checkout_url:string|null,payment_state,expires_at}`. Original checkout UUID is an unguessable capability, verified together with order ID. No personal data or secrets returned. No provider request triggered by this endpoint. Wrong key/order returns 404. A checkout URL is returned only for unpaid awaiting-payment orders with ready sessions and a deadline in the future.
- POST `/api/payments/teya/webhook` accepts exact raw JSON plus `x-teya-signature` Base64 RSA/SHA-256. This one POST route is exempt from browser CSRF headers. Invalid signatures return 401, mismatches fail, valid notifications return 204 only after database commit. Duplicate bodies do not create additional notifications. No success is inferred directly from the body. A worker verifies the session through the authenticated Teya API.
- GET `/api/admin/orders/{id}` additionally includes `online_payment:{state,expires_at,checked_at,last_error,cancellable,receipt}` for online orders. `receipt` is null before verified payment, otherwise `{transaction_id,session_id,amount_cents,currency,verified_at}` read from the persistent `payment_receipts` table. Late payments on closed orders retain this receipt alongside the review warning. Operational metadata and receipt identifiers are omitted from public creation, replay, account and payment-status responses. Credentials and checkout URLs are never in this object.
- Local payment states: `pending`, `ready`, `paid`, `failed`, `expired`, `review`. Order/payment state, receipt and email queue commit atomically. Persistent leases, provider idempotency UUIDs and unique transaction/session receipts protect restart and concurrent handling.
- Hosted checkout returns to `/?payment_return=<order UUID>`. This is a UI hint only. Original checkout capability remains in session storage, never in the return URL. The browser must fetch payment state and must not create a new payment when confirmation is delayed.

See [teya-payments.md](teya-payments.md) for credential storage, operational recovery, retry boundaries and required provider staging validation.

### Local online-order expiration

`ONLINE_PAYMENT_TIMEOUT_MINUTES` defaults to 60 and accepts integer 1..1440. On creation, `payment_attempts.expires_at` is persisted from order creation time. All online order responses include `payment_expires_at`. Settings changes and restarts do not extend existing deadlines. Migration of previously created online orders uses the original creation time plus one hour.

Independent maintenance every second expires due unpaid online orders, transactionally restoring reserved stock once and queuing one customer message. This runs even without Teya connectivity or configured credentials. Delivery-payment orders and verified paid orders are excluded. Status `expired` is terminal and cannot be assigned manually through admin PATCH. During application/database downtime, deadlines are processed after recovery.

Reconciliation checks the deadline while holding order/payment locks before accepting success. A late verified payment retains a unique receipt, leaves the order closed, sets payment process `review` and queues alerts once to the customer and shop. No further fulfilment or stock reservation occurs. Known provider sessions remain eligible for verification, but no new session is created after the local deadline. Teya's separate Hosted Checkout session lifetime remains 15 minutes.
