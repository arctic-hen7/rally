self.addEventListener('install', (event) => event.waitUntil(self.skipWaiting()));
self.addEventListener('activate', (event) => event.waitUntil(self.clients.claim()));
self.addEventListener('push', (event) => {
    const data = event.data?.json() || {};
    event.waitUntil(
        (async () => {
            const tabs = await self.clients.matchAll({ type: 'window', includeUncontrolled: true });
            if (tabs.some((tab) => tab.focused && new URL(tab.url).hash.includes(data.thread))) return;
            await self.registration.showNotification(data.title || 'Rally', {
                body: data.body,
                data,
                tag: data.run || data.thread,
                icon: '/icon-192.png',
            });
        })(),
    );
});
self.addEventListener('notificationclick', (event) => {
    event.notification.close();
    const data = event.notification.data;
    event.waitUntil(
        self.clients.openWindow(
            '/#thread/' +
                encodeURIComponent(data.thread) +
                (data.run ? '/run/' + encodeURIComponent(data.run) : ''),
        ),
    );
});
