import React, { useEffect, useRef, useState } from 'react';
import { Plus, TriangleAlert } from 'lucide-react';
import { api, Data, Form, Section } from './ui';

function httpOrigin(value?: string) {
    try {
        const url = new URL(value || '');
        if (url.protocol === 'http:') return url.origin;
    } catch {}
}

export function TransportWarning({ url, credentials = true }: { url?: string; credentials?: boolean }) {
    const origin = httpOrigin(url);
    if (!origin) return null;
    return (
        <div className="git-transport-warning" role="alert">
            <TriangleAlert aria-hidden="true" />
            <div>
                <strong>Unencrypted Git destination</strong>
                <p>
                    Rally will send{' '}
                    {credentials ? 'repository data and any Git credentials used here' : 'repository data'} to{' '}
                    <code>{origin}</code> over HTTP. Anyone able to intercept the connection could{' '}
                    {credentials
                        ? 'read the credentials or alter the Git traffic'
                        : 'read or alter the Git traffic'}
                    .
                </p>
            </div>
        </div>
    );
}

export function TransportLabel({ url }: { url?: string }) {
    return httpOrigin(url) ? <span className="git-transport-label">Unencrypted HTTP</span> : null;
}

function suggest(source: string) {
    const raw = source.trim();
    const scp = raw.match(/^(?:([\w.-]+)@)?([\w.-]+):(?!\d+(?:\/|$))\/?(.*)$/);
    const value =
        !raw.includes('://') && scp
            ? 'ssh://' + (scp[1] ? scp[1] + '@' : '') + scp[2] + '/' + scp[3]
            : raw.includes('://')
              ? raw
              : 'https://' + raw;
    try {
        const url = new URL(value);
        if (!['ssh:', 'https:'].includes(url.protocol) || url.password || url.search || url.hash) return;
        const path = url.pathname;
        const upstream = url.protocol === 'https:' ? url.href : 'https://' + url.hostname + path;
        const scope =
            path.endsWith('.git') || (!path.endsWith('/') && path.split('/').filter(Boolean).length >= 2)
                ? 'repository'
                : 'prefix';
        return { upstream, scope };
    } catch {}
}

export function RouteFields({ data, change }: { data: Data; change: (data: Data) => void }) {
    const edited = useRef({ upstream: !!data.id, scope: !!data.id });
    return (
        <>
            <label>
                Source Git URL
                <input
                    required
                    value={data.source || ''}
                    placeholder="git@example.org:team/"
                    onChange={(e) => change({ ...data, source: e.target.value })}
                    onBlur={() => {
                        const suggestion = suggest(data.source || '');
                        if (suggestion)
                            change({
                                ...data,
                                upstream: edited.current.upstream ? data.upstream : suggestion.upstream,
                                scope: edited.current.scope ? data.scope : suggestion.scope,
                            });
                    }}
                />
            </label>
            <label>
                Hosted destination URL
                <input
                    required
                    type="url"
                    value={data.upstream || ''}
                    placeholder="https://example.org/team/"
                    onChange={(e) => {
                        edited.current.upstream = true;
                        change({ ...data, upstream: e.target.value });
                    }}
                />
            </label>
            <TransportWarning url={data.upstream} />
            <label>
                Match
                <select
                    value={data.scope || 'prefix'}
                    onChange={(e) => {
                        edited.current.scope = true;
                        change({ ...data, scope: e.target.value });
                    }}
                >
                    <option value="prefix">Prefix (host or group)</option>
                    <option value="repository">Single repository</option>
                </select>
            </label>
        </>
    );
}

export function GitRoutes({ edit, revision }: { edit: (form: Form) => void; revision: number }) {
    const [rules, setRules] = useState<Data[]>([]);
    const [error, setError] = useState('');
    useEffect(() => {
        let live = true;
        api('routing.list')
            .then((r) => {
                if (live) setRules(r);
            })
            .catch((e) => {
                if (live) setError(e.message);
            });
        return () => {
            live = false;
        };
    }, [revision]);
    const open = (rule?: Data) =>
        edit({
            title: rule ? 'Edit repository URL' : 'Add private repository URL',
            op: rule ? 'routing.update' : 'routing.create',
            fields: [{ key: 'source', type: 'git-route', label: 'Source Git URL' }],
            initial: rule || { scope: 'prefix' },
            submit: 'Save rule',
        });
    return (
        <Section
            title="Private repository URLs"
            action={
                <button onClick={() => open()}>
                    <Plus /> Add URL
                </button>
            }
        >
            <small>
                Route matching Git requests from hosted workers through Rally. Each mission grants access to
                individual repositories.
            </small>
            {rules.map((rule) => (
                <div className="trigger-card" key={rule.id}>
                    <strong>{rule.source}</strong>
                    <small>
                        {rule.scope === 'prefix' ? 'Prefix' : 'Repository'} → {rule.upstream}{' '}
                        <TransportLabel url={rule.upstream} />
                    </small>
                    <div className="button-wrap">
                        <button onClick={() => open(rule)}>Edit</button>
                        <button
                            onClick={() =>
                                edit({
                                    title: 'Remove routing rule',
                                    op: 'routing.remove',
                                    initial: { id: rule.id },
                                    fields: [],
                                    submit: 'Remove ' + rule.source,
                                })
                            }
                        >
                            Remove
                        </button>
                    </div>
                </div>
            ))}
            {error && <p role="alert">{error}</p>}
        </Section>
    );
}

export const gitRequestHelp =
    'The URL of this repo matches an organisation routing rule, which may include public repos. Adding it will attach it read-only to this mission so Git can check it out using your credentials. Others in the mission will be able to implicitly read this repo, but not change it unless you add it fully to the mission as a workspace repo.';
