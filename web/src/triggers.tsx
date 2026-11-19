import React, { useEffect, useState } from 'react';
import { Clock3, Plus, X, Webhook } from 'lucide-react';
import { api, IconButton, required, Section } from './ui';

type Trigger = { cron?: string; timezone?: string; webhook?: string | null };
type Hook = { id: string; name: string };
function Schedule({ value, change }: { value: Trigger; change: (t: Trigger) => void }) {
    const parse = value.cron?.match(/^(\d+) (\d+) \* \* (\*|MON-FRI)$/);
    const [mode, setMode] = useState(parse ? (parse[3] === '*' ? 'Daily' : 'Weekdays') : 'Custom');
    const time = parse ? [parse[2], parse[1]].map((n) => n.padStart(2, '0')).join(':') : '09:00';
    const [preview, setPreview] = useState('');
    useEffect(() => {
        let live = true;
        const timer = setTimeout(
            () =>
                api('routine.preview', value)
                    .then((dates: string[]) => {
                        if (live)
                            setPreview(
                                dates
                                    .map((d) =>
                                        new Date(d).toLocaleString(undefined, {
                                            timeZone: value.timezone,
                                            weekday: 'short',
                                            hour: 'numeric',
                                            minute: '2-digit',
                                            day: 'numeric',
                                            month: 'short',
                                        }),
                                    )
                                    .join(' · '),
                            );
                    })
                    .catch((e) => {
                        if (live) setPreview(e.message);
                    }),
            180,
        );
        return () => {
            live = false;
            clearTimeout(timer);
        };
    }, [value.cron, value.timezone]);
    function setSchedule(mode: string, time: string) {
        setMode(mode);
        if (mode !== 'Custom') {
            const [hour, minute] = time.split(':').map(Number);
            change({ ...value, cron: minute + ' ' + hour + ' * * ' + (mode === 'Daily' ? '*' : 'MON-FRI') });
        }
    }
    return (
        <>
            <div className="trigger-grid">
                <label>
                    Repeat
                    <select value={mode} onChange={(e) => setSchedule(e.target.value, time)}>
                        {['Daily', 'Weekdays', 'Custom'].map((n) => (
                            <option key={n}>{n}</option>
                        ))}
                    </select>
                </label>
                {mode === 'Custom' ? (
                    <label>
                        Cron expression
                        <input
                            required
                            value={value.cron}
                            onChange={(e) => change({ ...value, cron: e.target.value })}
                        />
                    </label>
                ) : (
                    <label>
                        At
                        <input
                            type="time"
                            required
                            value={time}
                            onChange={(e) => setSchedule(mode, e.target.value)}
                        />
                    </label>
                )}
                <label>
                    Time zone
                    <input
                        required
                        value={value.timezone}
                        list="rally-timezones"
                        onChange={(e) => change({ ...value, timezone: e.target.value })}
                    />
                </label>
            </div>
            <small>
                {value.cron} · {preview}
            </small>
        </>
    );
}
export function TriggerEditor({ value, change }: { value: Trigger[]; change: (v: Trigger[]) => void }) {
    const [hooks, setHooks] = useState<Hook[]>([]);
    const [error, setError] = useState('');
    useEffect(() => {
        api('hook.list')
            .then(setHooks)
            .catch((e) => setError(e.message));
    }, []);
    const update = (i: number, t: Trigger) => change(value.map((old, j) => (i === j ? t : old)));
    return (
        <section className="trigger-list">
            <h3>Triggers</h3>
            <datalist id="rally-timezones">
                {['UTC', ...((Intl as any).supportedValuesOf?.('timeZone') || [])].map((z) => (
                    <option key={z} value={z} />
                ))}
            </datalist>
            {value.map((t, i) => (
                <div className="trigger-card" key={i}>
                    <header>
                        {t.webhook ? <Webhook /> : <Clock3 />}
                        <b>{t.webhook ? 'Webhook' : 'Schedule'}</b>
                        <span className="spacer" />
                        <IconButton
                            title="Remove trigger"
                            onClick={() => change(value.filter((_, j) => j !== i))}
                        >
                            <X />
                        </IconButton>
                    </header>
                    {t.webhook ? (
                        <label>
                            Endpoint
                            <select
                                value={t.webhook}
                                onChange={(e) => update(i, { webhook: e.target.value })}
                            >
                                {hooks.map((h) => (
                                    <option key={h.id} value={h.id}>
                                        {h.name}
                                    </option>
                                ))}
                            </select>
                        </label>
                    ) : (
                        <Schedule value={t} change={(v) => update(i, v)} />
                    )}
                </div>
            ))}
            <div className="button-wrap">
                <button
                    type="button"
                    onClick={() =>
                        change([
                            ...value,
                            {
                                cron: '0 9 * * *',
                                timezone: Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC',
                            },
                        ])
                    }
                >
                    <Plus />
                    Schedule
                </button>
                <button
                    type="button"
                    disabled={!hooks.length}
                    title={
                        hooks.length ? 'Add a webhook' : 'Create an endpoint under Routines → Webhooks first'
                    }
                    onClick={() => change([...value, { webhook: hooks[0].id }])}
                >
                    <Plus />
                    Webhook
                </button>
            </div>
            {error && <p role="alert">{error}</p>}
        </section>
    );
}

export function Webhooks({ edit, revision, act }: { edit: any; revision: number; act: any }) {
    const [hooks, setHooks] = useState<Hook[]>([]),
        [selected, select] = useState(''),
        [detail, setDetail] = useState<any>(null),
        [error, setError] = useState('');
    useEffect(() => {
        let live = true;
        Promise.all([api('hook.list'), selected ? api('hook.show', { hook: selected }) : null])
            .then(([list, d]) => {
                if (live) {
                    setHooks(list);
                    setDetail(d);
                }
            })
            .catch((e) => {
                if (live) setError(e.message);
            });
        return () => {
            live = false;
        };
    }, [selected, revision]);
    return (
        <Section
            title="Webhooks"
            action={
                <button
                    onClick={() =>
                        edit({
                            title: 'New webhook',
                            op: 'hook.create',
                            fields: [required('name', 'Webhook name')],
                            submit: 'Create webhook',
                        })
                    }
                >
                    <Plus />
                    New webhook
                </button>
            }
        >
            {hooks.map((h) => (
                <button
                    className="resource-row"
                    key={h.id}
                    aria-expanded={selected === h.id}
                    onClick={() => select(selected === h.id ? '' : h.id)}
                >
                    <Webhook />
                    {h.name}
                </button>
            ))}
            {detail?.id === selected && detail && (
                <div className="trigger-card">
                    <code>POST {detail.url}</code>
                    <h4>Sender tokens</h4>
                    {detail.tokens.map((t: any) => (
                        <div className="setting-line" key={t.id}>
                            {t.name}
                            <small>
                                {t.last_used
                                    ? 'Last used ' + new Date(t.last_used).toLocaleString()
                                    : 'Unused'}
                            </small>
                            <button onClick={() => act('hook.revoke', { hook: selected, token: t.id })}>
                                Revoke
                            </button>
                        </div>
                    ))}
                    <button
                        onClick={() =>
                            edit({
                                title: 'New sender token',
                                op: 'hook.token',
                                initial: { hook: selected },
                                fields: [required('name', 'Sender name')],
                                reveal: 'token',
                            })
                        }
                    >
                        Generate token
                    </button>
                    <h4>Subscribed routines</h4>
                    {detail.routines.map((r: Hook) => (
                        <p key={r.id}>{r.name}</p>
                    ))}
                    {!detail.routines.length && <small>No routines attached</small>}
                </div>
            )}
            {error && <p role="alert">{error}</p>}
        </Section>
    );
}
