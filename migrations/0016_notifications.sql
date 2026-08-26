SET search_path TO public;

CREATE TABLE notifications (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_key TEXT NOT NULL REFERENCES agents(agent_key) ON DELETE CASCADE,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    severity TEXT NOT NULL DEFAULT 'info' CHECK (severity IN ('info', 'warning', 'error')),
    status TEXT NOT NULL DEFAULT 'queued' CHECK (status IN ('queued', 'sent', 'failed')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    sent_at TIMESTAMPTZ,
    error TEXT
);

CREATE INDEX idx_notifications_agent_status
    ON notifications (agent_key, status);

CREATE INDEX idx_notifications_created_at
    ON notifications (created_at DESC);

CREATE OR REPLACE FUNCTION public.notify_notification_created()
RETURNS trigger AS $$
BEGIN
    PERFORM pg_notify('notification_created', NEW.id::text);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trigger_notifications_insert
    AFTER INSERT ON notifications
    FOR EACH ROW
    EXECUTE FUNCTION public.notify_notification_created();