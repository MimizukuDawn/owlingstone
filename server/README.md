# Owlingstone server (shared games and feedback)

A Cloudflare Worker (`src/worker.js`) with a D1 database (`schema.sql`), built for the **Free plan**:

| Free-plan limit | How it stays under it |
| --- | --- |
| 100,000 requests a day | one request carries up to 20 games and 5 feedback messages; stats are served by GitHub, not the Worker |
| 100,000 rows written a day | one row per game; above 80,000 writes a day the API answers 503 and apps keep their queue |
| 5 million rows read a day | limits use small counter tables (2 row reads per request); the daily stats job aggregates in SQL |
| 5 GB storage | games older than 60 days are deleted by the daily stats job |

With no payment method on the account, Cloudflare can never bill: past a limit, requests fail until the
next day (00:00 UTC).

## Routes

- `POST /v1/games` `{"install", "games": [...], "feedback": [...]}`: every field is checked by
  `src/validate.js`; anything not listed there is dropped, anything invalid refused.
- `DELETE /v1/installs/<id>`: deletes everything sent by that install id.
- `GET /v1/admin/feedback`, `POST /v1/admin/feedback/done`: maintainer only, `authorization: Bearer <ADMIN_KEY>`.

## Deploy and stats

`.github/workflows/deploy-api.yml` deploys on every change under `server/` (repository secrets
`CLOUDFLARE_API_TOKEN`, `CLOUDFLARE_ACCOUNT_ID`, `OWLINGSTONE_ADMIN_KEY`). `.github/workflows/stats.yml`
runs `build_stats.py` once a day and publishes `stats.json` on the `stats` pre-release.

## Test locally

```bash
cd server
npx wrangler d1 execute owlingstone --local --file schema.sql
echo "ADMIN_KEY=$(python3 -c 'import secrets; print(secrets.token_hex(24))')" > .dev.vars
npx wrangler dev --local --port 8799 &
test/smoke.sh http://localhost:8799
```
