SET search_path TO public;

DROP TRIGGER IF EXISTS trigger_notifications_insert ON notifications;
DROP FUNCTION IF EXISTS public.notify_notification_created();
DROP INDEX IF EXISTS idx_notifications_created_at;
DROP INDEX IF EXISTS idx_notifications_agent_status;
DROP TABLE IF EXISTS notifications;