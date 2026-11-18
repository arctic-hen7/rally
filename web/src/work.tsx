import React, { useEffect, useState } from 'react';
import {
    ArrowUp,
    Check,
    CircleHelp,
    LoaderCircle,
    OctagonPause,
    PanelRightClose,
    Radio,
    X,
} from 'lucide-react';
import { api, Body, IconButton } from './ui';

function Questions({ q, run, changed }: any) {
    const [answers, setAnswers] = useState<Record<string, string>>({}),
        [error, setError] = useState(''),
        [pending, setPending] = useState(false);
    const native = Array.isArray(q.prompt) ? q.prompt : q.prompt.questions;
    const schema = q.prompt.requestedSchema || q.prompt.schema;
    const list =
        native ||
        Object.entries(schema?.properties || {}).map(([id, v]: any) => ({
            id,
            question: v.title || v.description || id,
            options: v.enum,
        }));
    const fields = list?.length ? list : [{ id: 'answer', question: q.prompt.message || 'Your answer' }];
    return (
        <form
            className="question"
            onSubmit={async (e) => {
                e.preventDefault();
                setPending(true);
                try {
                    const values = Object.fromEntries(
                        fields.map((f: any, i: number) => [
                            f.id || String(i),
                            schema ? answers[f.id || String(i)] : { answers: [answers[f.id || String(i)]] },
                        ]),
                    );
                    await api('run.answer', { run, question: q.id, answer: values });
                    changed();
                } catch (e: any) {
                    setError(e.message);
                } finally {
                    setPending(false);
                }
            }}
        >
            <strong>
                <CircleHelp size={16} />
                {q.answer ? 'Answered' : q.blocking ? 'Waiting for an answer' : 'Question'}
            </strong>
            {fields.map((f: any, i: number) => {
                const key = f.id || String(i);
                return (
                    <label key={key}>
                        {f.question || f.title}
                        {q.answer ? (
                            <small>{JSON.stringify(q.answer[key]?.answers || q.answer[key])}</small>
                        ) : (
                            <>
                                <div className="button-wrap">
                                    {f.options?.map((o: any) => (
                                        <button
                                            type="button"
                                            className={answers[key] === (o.label || o) ? 'chosen' : ''}
                                            title={o.description}
                                            key={o.label || o}
                                            onClick={() => setAnswers({ ...answers, [key]: o.label || o })}
                                        >
                                            {o.label || o}
                                        </button>
                                    ))}
                                </div>
                                <input
                                    required
                                    value={answers[key] || ''}
                                    placeholder="Your answer"
                                    onChange={(e) => setAnswers({ ...answers, [key]: e.target.value })}
                                />
                            </>
                        )}
                    </label>
                );
            })}
            {!q.answer && (
                <button className="primary" disabled={pending}>
                    Send answer
                </button>
            )}
            {error && <p className="error">{error}</p>}
        </form>
    );
}
export function Working({ id, close, tick, refresh }: any) {
    const [run, setRun] = useState<any>(null),
        [body, setBody] = useState(''),
        [error, setError] = useState(''),
        [busy, setBusy] = useState(false);
    async function load() {
        try {
            setRun(await api('run.show', { run: id }));
        } catch (e: any) {
            setError(e.message);
        }
    }
    useEffect(() => {
        let alive = true;
        async function get() {
            try {
                const r = await api('run.show', { run: id });
                if (alive) setRun(r);
            } catch (e: any) {
                if (alive) setError(e.message);
            }
        }
        get();
        const t = setInterval(get, 900);
        return () => {
            alive = false;
            clearInterval(t);
        };
    }, [id]);
    if (!run)
        return (
            <aside className="side-pane">
                <header className="pane-head">
                    Agent activity
                    <IconButton title="Close" onClick={close}>
                        <X />
                    </IconButton>
                </header>
                <p>{error || 'Connecting…'}</p>
            </aside>
        );
    const active = ['working', 'queued', 'interrupted', 'stopping'].includes(run.status);
    return (
        <aside className="side-pane working">
            <header className="pane-head">
                <Radio size={16} />
                <b>Agent activity</b>
                <IconButton title="Close activity" onClick={close}>
                    <PanelRightClose />
                </IconButton>
            </header>
            <div className="pane-content">
                <h2>@{run.name}</h2>
                <div className="run-status">
                    <i className={active ? 'pulse' : ''} />
                    {run.status} <span>· {run.model}</span>
                </div>
                {run.partial && <Body text={run.partial} />}{' '}
                {run.error && <p className="error">{run.error}</p>}
                {run.questions.map((q: any) => (
                    <Questions key={q.id} q={q} run={id} changed={load} />
                ))}
            </div>
            {active && (
                <form
                    className="steering"
                    onSubmit={async (e) => {
                        e.preventDefault();
                        setBusy(true);
                        try {
                            await api('run.steer', { run: id, body });
                            setBody('');
                            refresh();
                        } catch (e: any) {
                            setError(e.message);
                        } finally {
                            setBusy(false);
                        }
                    }}
                >
                    {error && <p className="error">{error}</p>}
                    <textarea
                        placeholder={'Steer @' + run.name + '…'}
                        required
                        value={body}
                        onChange={(e) => setBody(e.target.value)}
                    />
                    <div>
                        <button
                            type="button"
                            onClick={async () => {
                                try {
                                    await api('run.stop', { run: id });
                                    load();
                                } catch (e: any) {
                                    setError(e.message);
                                }
                            }}
                        >
                            <OctagonPause />
                            Stop
                        </button>
                        <button className="primary" disabled={!body.trim() || busy}>
                            <ArrowUp />
                            Steer
                        </button>
                    </div>
                </form>
            )}
        </aside>
    );
}
