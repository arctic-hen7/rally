import React, { useState, useEffect } from 'react';
import { Eye, EyeOff, KeyRound, LockKeyhole, Plus, X } from 'lucide-react';
import { api, IconButton, Section } from './ui';

export function ModelSettings() {
    const [model, setModel] = useState(''),
        [provider, setProvider] = useState<any>(null),
        [message, setMessage] = useState('');
    useEffect(() => {
        api('models.settings')
            .then((r) => {
                setModel(r.model);
                setProvider(r.provider);
            })
            .catch((e) => setMessage(e.message));
    }, []);
    return (
        <Section title="Models">
            <form
                className="form"
                onSubmit={async (e) => {
                    e.preventDefault();
                    try {
                        await api('models.settings', { model });
                        setMessage('Saved. The next WIP summary uses this model.');
                    } catch (err: any) {
                        setMessage(err.message);
                    }
                }}
            >
                <label>
                    WIP summary model
                    <input required value={model} onChange={(e) => setModel(e.target.value)} />
                </label>
                <button>Save model</button>
                {provider?.error && <p role="alert">{provider.error}</p>}
                {message && <small role="status">{message}</small>}
            </form>
        </Section>
    );
}

export function SecretsPanel({ target, onAdded }: { target: string; onAdded?: () => void }) {
    const [open, setOpen] = useState(false),
        [entries, setEntries] = useState<Record<string, any>>({}),
        [error, setError] = useState(''),
        [editing, setEditing] = useState<string | null>(null),
        [name, setName] = useState(''),
        [value, setValue] = useState(''),
        [revealed, reveal] = useState(false),
        [busy, setBusy] = useState(false);
    useEffect(() => {
        let live = true;
        if (open)
            api('secrets.list', { target })
                .then((v) => {
                    if (live) setEntries(v);
                })
                .catch((e) => {
                    if (live) setError(e.message);
                });
        return () => {
            live = false;
        };
    }, [target, open]);
    function begin(key: string) {
        setEditing(key);
        setName(key);
        setValue(entries[key]?.value ?? '');
        reveal(false);
        setError('');
    }
    async function apply(key: string, value: string | null) {
        setBusy(true);
        setError('');
        try {
            const old = entries[key];
            const result = await api('secrets.apply', {
                target,
                changes: { [key]: { value, base: old?.checksum ?? null, source: old?.source ?? null } },
            });
            setEntries(result);
            setEditing(null);
            setValue('');
            if (!old && value !== null) onAdded?.();
        } catch (e: any) {
            setError(e.message);
        } finally {
            setBusy(false);
        }
    }
    return (
        <Section
            title="Secrets"
            action={
                <button
                    aria-expanded={open}
                    onClick={() => {
                        setOpen(!open);
                        setEntries({});
                        setEditing(null);
                        setValue('');
                        setError('');
                    }}
                >
                    <KeyRound />
                    {open ? 'Close' : 'Manage secrets'}
                </button>
            }
        >
            {open && (
                <>
                    {Object.entries(entries).map(([key, s]) => (
                        <div key={key} className="secret-row">
                            <div>
                                <code>{key}</code>
                                <small>
                                    {s.conflict
                                        ? 'Conflicting skills — set an override'
                                        : s.sources?.length > 1
                                          ? 'Overrides ' +
                                            s.sources
                                                .slice(0, -1)
                                                .map((x: any) => x.name)
                                                .join(', ')
                                          : s.source !== target
                                            ? 'From ' + s.sources?.[0]?.name
                                            : '••••••••'}
                                </small>
                            </div>
                            <button onClick={() => begin(key)}>
                                {s.source === target ? 'Edit' : 'Override'}
                            </button>
                        </div>
                    ))}
                    {editing === null ? (
                        <button onClick={() => begin('')}>
                            <Plus />
                            Add secret
                        </button>
                    ) : (
                        <form
                            className="form secret-edit"
                            onSubmit={(e) => {
                                e.preventDefault();
                                void apply(name, value);
                            }}
                        >
                            <label>
                                Environment variable
                                <input
                                    required
                                    readOnly={!!editing}
                                    pattern="[a-zA-Z_][a-zA-Z_0-9]*"
                                    value={name}
                                    onChange={(e) => setName(e.target.value)}
                                />
                            </label>
                            <label>
                                Secret value
                                {revealed ? (
                                    <textarea value={value} onChange={(e) => setValue(e.target.value)} />
                                ) : (
                                    <input
                                        type="password"
                                        autoComplete="new-password"
                                        value={value}
                                        onChange={(e) => setValue(e.target.value)}
                                    />
                                )}
                            </label>
                            <div className="button-wrap">
                                <button type="button" onClick={() => reveal(!revealed)}>
                                    {revealed ? <EyeOff /> : <Eye />}
                                    {revealed ? 'Hide' : 'Reveal'}
                                </button>
                                <button className="primary" disabled={busy || (!editing && !!entries[name])}>
                                    Save secret
                                </button>
                                <button
                                    type="button"
                                    onClick={() => {
                                        setEditing(null);
                                        setValue('');
                                    }}
                                >
                                    Cancel
                                </button>
                                {editing && entries[editing]?.source === target && (
                                    <button type="button" onClick={() => apply(editing, null)}>
                                        Delete secret
                                    </button>
                                )}
                            </div>
                        </form>
                    )}
                    {error && <p role="alert">{error}</p>}
                </>
            )}
        </Section>
    );
}

export function ResourceAccess({
    resource,
    kind,
    data,
    refresh,
}: {
    resource: any;
    kind: 'skill' | 'agent';
    data: any;
    refresh: () => void;
}) {
    const [access, setAccess] = useState<any>(null),
        [principal, setPrincipal] = useState(''),
        [error, setError] = useState(''),
        [suggest, setSuggest] = useState(false),
        [widen, confirmWiden] = useState(false);
    const target = resource.id;
    useEffect(() => {
        let live = true;
        api('resource.privacy', { target })
            .then((v) => {
                if (live) setAccess(v);
            })
            .catch((e) => {
                if (live) setError(e.message);
            });
        return () => {
            live = false;
        };
    }, [target, resource]);
    async function save(fields: any) {
        try {
            setAccess(await api('resource.privacy', { target, ...fields }));
            setSuggest(false);
            confirmWiden(false);
            setError('');
            refresh();
        } catch (e: any) {
            setError(e.message);
        }
    }
    const people = [...data.people, ...data.groups];
    return (
        <>
            <Section title="Access">
                {access && (
                    <>
                        <label className="check-field">
                            <input
                                type="checkbox"
                                checked={access.private}
                                onChange={(e) => {
                                    if (!e.target.checked && kind === 'agent') confirmWiden(true);
                                    else void save({ private: e.target.checked });
                                }}
                            />
                            Make this {kind} private
                        </label>
                        {widen && (
                            <div className="notice">
                                <span>
                                    Everyone in the organisation will be able to use this agent, its attached
                                    private skills, and any secrets the agent has.
                                </span>
                                <button onClick={() => save({ private: false, confirm: true })}>
                                    Make organisation-wide
                                </button>
                                <button onClick={() => confirmWiden(false)}>Cancel</button>
                            </div>
                        )}
                        {access.private && (
                            <>
                                <small>Invite people or groups to use this {kind} in new missions.</small>
                                {access.invites.map((id: string) => (
                                    <div className="setting-line" key={id}>
                                        {people.find((p: any) => p.id === id)?.name ?? id}
                                        <IconButton
                                            title="Remove invitation"
                                            onClick={() =>
                                                save({
                                                    invites: access.invites.filter((s: string) => s !== id),
                                                })
                                            }
                                        >
                                            <X />
                                        </IconButton>
                                    </div>
                                ))}
                                <div className="button-wrap">
                                    <select
                                        aria-label="Invite person or group"
                                        value={principal}
                                        onChange={(e) => setPrincipal(e.target.value)}
                                    >
                                        <option value="">Choose person or group</option>
                                        {people
                                            .filter((p: any) => !access.invites.includes(p.id))
                                            .map((p: any) => (
                                                <option value={p.id} key={p.id}>
                                                    {p.name}
                                                </option>
                                            ))}
                                    </select>
                                    <button
                                        disabled={!principal}
                                        onClick={() => {
                                            void save({ invites: [...access.invites, principal] });
                                            setPrincipal('');
                                        }}
                                    >
                                        Invite
                                    </button>
                                </div>
                            </>
                        )}
                        <h4>{kind === 'skill' ? 'Attached to' : 'Invited to'}</h4>
                        {access.targets.length ? (
                            access.targets.map((t: any) => (
                                <div className="setting-line" key={t.id}>
                                    <span>{t.name}</span>
                                    <small>{t.id}</small>
                                    {kind === 'skill' && (
                                        <button
                                            onClick={async () => {
                                                try {
                                                    await api('skill.detach', {
                                                        skill: target,
                                                        target: t.id,
                                                    });
                                                    refresh();
                                                } catch (e: any) {
                                                    setError(e.message);
                                                }
                                            }}
                                        >
                                            Detach
                                        </button>
                                    )}
                                </div>
                            ))
                        ) : (
                            <small>No attachments yet</small>
                        )}
                    </>
                )}
                {error && <p role="alert">{error}</p>}
            </Section>
            <SecretsPanel
                key={target}
                target={target}
                onAdded={() => {
                    if (!access?.private) setSuggest(true);
                }}
            />
            {suggest && (
                <div className="notice" role="status">
                    <LockKeyhole />
                    <span>This {kind} contains secrets and is organisation-wide.</span>
                    <button onClick={() => save({ private: true })}>Make private</button>
                    <IconButton title="Dismiss" onClick={() => setSuggest(false)}>
                        <X />
                    </IconButton>
                </div>
            )}
        </>
    );
}
