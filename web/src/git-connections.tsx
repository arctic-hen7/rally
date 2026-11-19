import React, { useEffect, useState } from 'react';
import { GitBranch, Plus } from 'lucide-react';
import { api, Form, required, Section } from './ui';

type Connection = { id: string; name: string; username: string; origin: string };
type Usage = { type: string; target: string; name: string };
export function GitAccess({
    remote,
    connection,
    confirmed,
    share,
    pending,
    change,
}: {
    remote: string;
    connection?: string;
    confirmed?: boolean;
    share?: boolean;
    pending?: boolean;
    change: (connection: string, confirm: boolean) => void;
}) {
    const [list, setList] = useState<Connection[]>([]);
    const [error, setError] = useState('');
    useEffect(() => {
        api('connection.list')
            .then(setList)
            .catch((e) => setError(e.message));
    }, []);
    let origin = '';
    try {
        origin = new URL(remote).origin;
    } catch {}
    const matches = list.filter((c) => c.origin === origin);
    return (
        <div className="editor-field">
            <label>
                Git connection
                <select
                    required={pending}
                    value={pending ? '' : connection || 'anonymous'}
                    onChange={(e) => change(e.target.value === 'anonymous' ? '' : e.target.value, false)}
                >
                    {pending && (
                        <option value="" disabled>
                            Choose access to reconnect
                        </option>
                    )}
                    <option value="anonymous">Anonymous · read only</option>
                    {matches.map((c) => (
                        <option key={c.id} value={c.id}>
                            {c.name} · {c.username}
                        </option>
                    ))}
                </select>
            </label>
            {share && connection && (
                <label className="check-field">
                    <input
                        type="checkbox"
                        required
                        checked={!!confirmed}
                        onChange={(e) => change(connection, e.target.checked)}
                    />
                    Allow current and future mission members and their hosted agents to fetch and push to this
                    repository using my connection.
                </label>
            )}
            {error && <small role="alert">{error}</small>}
        </div>
    );
}

export function GitConnections({ revision, edit }: { revision: number; edit: (f: Form) => void }) {
    const [list, setList] = useState<Connection[]>([]),
        [selected, select] = useState(''),
        [uses, setUses] = useState<Usage[]>([]),
        [error, setError] = useState('');
    useEffect(() => {
        let live = true;
        api('connection.list')
            .then((v) => {
                if (live) setList(v);
            })
            .catch((e) => {
                if (live) setError(e.message);
            });
        return () => {
            live = false;
        };
    }, [revision]);
    useEffect(() => {
        let live = true;
        setUses([]);
        if (selected)
            api('connection.usage', { connection: selected })
                .then((v) => {
                    if (live) setUses(v);
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
            title="Git connections"
            action={
                <button
                    onClick={() =>
                        edit({
                            title: 'New Git connection',
                            op: 'connection.create',
                            fields: [
                                required('name', 'Connection name'),
                                required('host', 'HTTPS forge host'),
                                required('username', 'Git username'),
                                required('token', 'Access token', 'password'),
                            ],
                            submit: 'Save connection',
                        })
                    }
                >
                    <Plus />
                    New connection
                </button>
            }
        >
            {list.length === 0 && <small>Connect an HTTPS Git account to use private repositories.</small>}
            {list.map((c) => (
                <React.Fragment key={c.id}>
                    <button
                        className="resource-row"
                        aria-expanded={selected === c.id}
                        onClick={() => select(selected === c.id ? '' : c.id)}
                    >
                        <GitBranch />
                        {c.name}
                        <small>
                            {c.username} · {c.origin}
                        </small>
                    </button>
                    {selected === c.id && (
                        <div className="trigger-card">
                            <strong>Used by</strong>
                            {uses.length === 0 && <small>No attachments</small>}
                            {uses.map((u) => (
                                <div key={u.type + u.target + u.name}>
                                    {u.type} · {u.name} <small>{u.target}</small>
                                </div>
                            ))}
                            <div className="button-wrap">
                                <button
                                    onClick={() =>
                                        edit({
                                            title: 'Replace Git token',
                                            op: 'connection.update',
                                            initial: { connection: c.id },
                                            fields: [required('token', 'New access token', 'password')],
                                        })
                                    }
                                >
                                    Replace token
                                </button>
                                <button
                                    onClick={() =>
                                        edit({
                                            title: 'Disconnect ' + c.name,
                                            op: 'connection.remove',
                                            initial: { connection: c.id },
                                            fields: [
                                                {
                                                    key: 'confirm',
                                                    type: 'checkbox',
                                                    required: true,
                                                    label: 'Pause repositories, routines and imported skills using this connection.',
                                                },
                                            ],
                                            submit: 'Disconnect',
                                        })
                                    }
                                >
                                    Disconnect
                                </button>
                            </div>
                        </div>
                    )}
                </React.Fragment>
            ))}
            {error && <p role="alert">{error}</p>}
        </Section>
    );
}
