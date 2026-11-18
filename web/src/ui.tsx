import React, { useEffect, useRef, useState } from 'react';
import { ArrowUpRight, Check, ChevronDown, Copy, LoaderCircle, X } from 'lucide-react';
import Markdown from 'react-markdown';
import remarkGfm from 'remark-gfm';

export type Data = Record<string, any>;
export async function api(op: string, data: Data = {}, auth = false) {
    const response = await fetch(`/${auth ? 'auth' : 'api'}/${op}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(data),
    });
    const result = await response.json();
    if (!response.ok) throw new Error(result.error || response.statusText);
    return result;
}

export function Brand() {
    return <img className="brand-mark" src="/rally.svg" alt="" />;
}
export function Avatar({
    name,
    small = false,
    agent = false,
}: {
    name: string;
    small?: boolean;
    agent?: boolean;
}) {
    const hash = [...name].reduce((n, c) => n + c.charCodeAt(0), 0);
    const colors = ['plum', 'sage', 'sand', 'blue', 'clay'];
    return (
        <span className={`avatar ${small ? 'small' : ''} ${agent ? 'agent' : colors[hash % colors.length]}`}>
            {agent
                ? '✳'
                : name
                      .trim()
                      .split(/\s+/)
                      .slice(0, 2)
                      .map((s) => s[0]?.toUpperCase())
                      .join('')}
        </span>
    );
}
export function Body({ text }: { text: string }) {
    return (
        <div className="markdown">
            <Markdown
                remarkPlugins={[remarkGfm]}
                components={{
                    a: (props) => <a {...props} rel="noreferrer" target="_blank" />,
                }}
            >
                {text}
            </Markdown>
        </div>
    );
}
export function IconButton({ title, children, onClick, disabled = false }: any) {
    return (
        <button
            type="button"
            className="icon-button"
            title={title}
            aria-label={title}
            disabled={disabled}
            onClick={onClick}
        >
            {children}
        </button>
    );
}
export function Modal({ title, onClose, children, wide = false }: any) {
    const ref = useRef<HTMLDialogElement>(null);
    useEffect(() => {
        ref.current?.showModal();
    }, []);
    return (
        <dialog
            ref={ref}
            className={wide ? 'dialog wide' : 'dialog'}
            onCancel={onClose}
            onClick={(e) => {
                if (e.target === ref.current) {
                    const r = ref.current!.getBoundingClientRect();
                    if (
                        e.clientX < r.left ||
                        e.clientX > r.right ||
                        e.clientY < r.top ||
                        e.clientY > r.bottom
                    )
                        onClose();
                }
            }}
        >
            <div className="dialog-head">
                <h2>{title}</h2>
                <IconButton title="Close" onClick={onClose}>
                    <X />
                </IconButton>
            </div>
            {children}
        </dialog>
    );
}
export type Field = {
    key: string;
    label: string;
    type?: string;
    placeholder?: string;
    required?: boolean;
    options?: { value: string; label: string }[];
};
export type Form = {
    title: string;
    fields: Field[];
    op?: string;
    initial?: Data;
    submit?: string;
    save?: (d: Data) => Promise<any>;
    reveal?: string;
    after?: (d: Data) => void;
};
export function Editor({ form, close, changed }: { form: Form; close: () => void; changed: () => void }) {
    const [data, setData] = useState<Data>({
        ...Object.fromEntries(
            form.fields.filter((f) => f.options?.length).map((f) => [f.key, f.options![0].value]),
        ),
        ...form.initial,
    });
    const [busy, setBusy] = useState(false),
        [error, setError] = useState(''),
        [secret, setSecret] = useState('');
    async function submit(e: React.FormEvent) {
        e.preventDefault();
        setError('');
        setBusy(true);
        try {
            const send = { ...data };
            for (const f of form.fields) if (f.type === 'json') send[f.key] = JSON.parse(send[f.key] || '[]');
            const r = form.save ? await form.save(send) : await api(form.op!, send);
            changed();
            form.after?.(r);
            if (form.reveal && r[form.reveal]) setSecret(r[form.reveal]);
            else close();
        } catch (e: any) {
            setError(e.message);
        } finally {
            setBusy(false);
        }
    }
    return (
        <Modal title={form.title} onClose={close}>
            {secret ? (
                <div className="form">
                    <p>Copy this value before closing.</p>
                    <code className="secret">{secret}</code>
                    <button onClick={() => navigator.clipboard.writeText(secret)}>
                        <Copy />
                        Copy
                    </button>
                </div>
            ) : (
                <form className="form" onSubmit={submit}>
                    {form.fields.map((f) => (
                        <label key={f.key}>
                            {f.label}
                            {f.options ? (
                                <select
                                    value={data[f.key] ?? ''}
                                    onChange={(e) => setData({ ...data, [f.key]: e.target.value })}
                                >
                                    {f.options.map((o) => (
                                        <option key={o.value} value={o.value}>
                                            {o.label}
                                        </option>
                                    ))}
                                </select>
                            ) : f.type === 'area' || f.type === 'json' ? (
                                <textarea
                                    required={f.required}
                                    placeholder={f.placeholder}
                                    rows={5}
                                    value={data[f.key] ?? ''}
                                    onChange={(e) => setData({ ...data, [f.key]: e.target.value })}
                                />
                            ) : (
                                <input
                                    required={f.required}
                                    type={f.type || 'text'}
                                    placeholder={f.placeholder}
                                    value={data[f.key] ?? ''}
                                    onChange={(e) => setData({ ...data, [f.key]: e.target.value })}
                                />
                            )}
                        </label>
                    ))}
                    {error && (
                        <p role="alert" className="error">
                            {error}
                        </p>
                    )}
                    <div className="form-actions">
                        <button type="button" onClick={close}>
                            Cancel
                        </button>
                        <button className="primary" disabled={busy}>
                            {busy ? <LoaderCircle className="spin" /> : <Check />}
                            {form.submit || 'Save'}
                        </button>
                    </div>
                </form>
            )}
        </Modal>
    );
}
export function Auth({ done }: { done: () => void }) {
    const [status, setStatus] = useState<Data>({}),
        [mode, setMode] = useState('login'),
        [error, setError] = useState(''),
        [busy, setBusy] = useState(false);
    useEffect(() => {
        api('status', {}, true)
            .then((s) => {
                setStatus(s);
                if (s.setup) setMode('bootstrap');
            })
            .catch((e) => setError(e.message));
    }, []);
    const title =
        mode === 'bootstrap'
            ? 'Create your workspace'
            : mode === 'register'
              ? 'Join your team'
              : 'Welcome back';
    return (
        <main className="auth-page">
            <div className="auth-card">
                <div className="brand auth-brand">
                    <Brand /> Rally
                </div>
                <h1>{title}</h1>
                <form
                    className="form"
                    onSubmit={async (e) => {
                        e.preventDefault();
                        setError('');
                        setBusy(true);
                        try {
                            await api(mode, Object.fromEntries(new FormData(e.currentTarget)), true);
                            done();
                        } catch (e: any) {
                            setError(e.message);
                        } finally {
                            setBusy(false);
                        }
                    }}
                >
                    {mode !== 'login' && (
                        <label>
                            Display name
                            <input name="name" autoComplete="name" required autoFocus />
                        </label>
                    )}
                    <label>
                        Email
                        <input
                            name="email"
                            type="email"
                            autoComplete="username"
                            autoFocus={mode === 'login'}
                            required
                        />
                    </label>
                    <label>
                        Password
                        <input
                            name="password"
                            type="password"
                            autoComplete={mode === 'login' ? 'current-password' : 'new-password'}
                            required
                            minLength={mode === 'login' ? 1 : 10}
                        />
                    </label>
                    {mode !== 'login' && (
                        <label>
                            {mode === 'bootstrap' ? 'Server bootstrap secret' : 'Invitation code'}
                            <input name="code" type="password" required />
                        </label>
                    )}
                    {error && (
                        <p role="alert" className="error">
                            {error}
                        </p>
                    )}
                    <button className="primary" disabled={busy}>
                        {busy ? 'Please wait…' : mode === 'login' ? 'Sign in' : 'Create account'}
                        <ArrowUpRight />
                    </button>
                </form>
                {status.oidc && (
                    <a className="button" href="/auth/oidc/start">
                        Continue with single sign-on
                    </a>
                )}
                {!status.setup && (
                    <button
                        className="text-button"
                        onClick={() => setMode(mode === 'login' ? 'register' : 'login')}
                    >
                        {mode === 'login' ? 'Use an invitation' : 'Back to sign in'}
                    </button>
                )}
            </div>
        </main>
    );
}
export function Section({ title, children, action }: any) {
    return (
        <section className="detail-section">
            <header>
                <h3>{title}</h3>
                {action}
            </header>
            {children}
        </section>
    );
}
export const options = (rows: any[], value = 'id') =>
    rows.map((r) => ({ value: r[value], label: r.name || r.title || r[value] }));
export const required = (key: string, label: string, type = 'text'): Field => ({
    key,
    label,
    type,
    required: true,
});
export const choice = (key: string, label: string, items: any[]): Field => ({
    key,
    label,
    options: items,
    required: true,
});
