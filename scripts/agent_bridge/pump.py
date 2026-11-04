import asyncio
import time


class EventPump:
    """Reuse existing leases; delivery adapter success alone is never an Agent acknowledgement."""

    def __init__(self, client, sink, consumer, lease_seconds=30):
        if not 5 <= lease_seconds <= 300:
            raise ValueError('lease_seconds must be 5..300')
        self.client, self.sink, self.consumer = client, sink, consumer
        self.lease_seconds = lease_seconds

    async def deliver(self, event, receipt_timeout=120):
        lease = {'event_id': event['event_id'], 'consumer': self.consumer, 'lease_token': event['lease_token']}
        deadline = time.monotonic() + receipt_timeout
        renew_at = time.monotonic() + self.lease_seconds / 3
        notify = asyncio.create_task(self.sink.notify(self.client.account, event))
        identifier = None
        try:
            while True:
                if notify.done():
                    identifier = notify.result()
                if identifier is not None and self.sink.receipt(identifier):
                    await self.client.tool('ack_event', **lease)
                    return {'status': 'acknowledged', 'delivery_id': identifier, 'event_id': event['event_id']}
                if time.monotonic() >= deadline:
                    # Leave it leased: the receiver might still be working. Expiry permits recovery.
                    return {'status': 'awaiting_agent_receipt', 'delivery_id': identifier, 'event_id': event['event_id']}
                if time.monotonic() >= renew_at:
                    await self.client.tool('renew_event_lease', **lease, lease_seconds=self.lease_seconds)
                    renew_at = time.monotonic() + self.lease_seconds / 3
                await asyncio.sleep(0.2)
        finally:
            if not notify.done():
                notify.cancel()
            await asyncio.gather(notify, return_exceptions=True)

    async def claim(self, recipient=None, expected_preview=None):
        claimed = await self.client.tool('claim_events', consumer=self.consumer, limit=100,
                                         lease_seconds=self.lease_seconds)
        selected = None
        for event in claimed['events']:
            conversation = event.get('payload', {}).get('conversation', {})
            matches = ((recipient is None or conversation.get('display_name') == recipient) and
                       (expected_preview is None or ''.join(expected_preview.split()) in
                        ''.join(conversation.get('preview', '').split())))
            if matches and selected is None:
                selected = event
            else:
                await self.client.tool('release_event', event_id=event['event_id'],
                                       consumer=self.consumer, lease_token=event['lease_token'])
        return selected
