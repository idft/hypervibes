import { queryOptions } from "@tanstack/react-query";
import { getNotification, getNotificationSettings, listNotifications } from "./notifications.js";

export const notificationKeys = { all: (agentId: string) => ["notifications", agentId] as const };
export const notificationsQueryOptions = (agentId: string, page: number) => queryOptions({
  queryKey: [...notificationKeys.all(agentId), "list", page],
  queryFn: () => listNotifications({ data: { agentId, page } }),
  staleTime: 0,
});
export const notificationQueryOptions = (agentId: string, id: string) => queryOptions({
  queryKey: [...notificationKeys.all(agentId), "detail", id],
  queryFn: () => getNotification({ data: { agentId, id } }),
  staleTime: 0,
});
export const notificationSettingsQueryOptions = (agentId: string) => queryOptions({
  queryKey: ["notification-settings", agentId],
  queryFn: () => getNotificationSettings({ data: { agentId } }),
});
