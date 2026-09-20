-- A webhook delivery used to be POSTed with its row still locked inside
-- the selecting transaction: up to 20 endpoints x a 10s timeout on one
-- pool connection, holding row locks the whole time. A handful of slow
-- receivers could pin connections long enough to starve the API itself.
--
-- Splitting that into "claim, POST, record" needs a state for a row that
-- has been claimed by a worker but has no outcome yet. Without it a crash
-- between the claim and the result would strand the row as `pending` and
-- it would be delivered twice, or strand it as `failed` and it would never
-- be delivered at all.
--
-- `claimed_at` is what makes the state recoverable: a worker that dies
-- mid-batch leaves in_flight rows behind, and the next pass reclaims any
-- older than the claim timeout back to pending.
ALTER TABLE webhook_deliveries
    DROP CONSTRAINT webhook_deliveries_status_check;

ALTER TABLE webhook_deliveries
    ADD CONSTRAINT webhook_deliveries_status_check
    CHECK (status IN ('pending', 'in_flight', 'delivered', 'failed'));

ALTER TABLE webhook_deliveries
    ADD COLUMN claimed_at timestamptz;

-- Reclaim scans look only at in-flight rows.
CREATE INDEX webhook_deliveries_in_flight
    ON webhook_deliveries (claimed_at)
    WHERE status = 'in_flight';
