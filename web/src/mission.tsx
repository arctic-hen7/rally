import { gitRequestHelp, TransportLabel } from './git-routing';
import { SecretsPanel } from './access';
import React, { useEffect, useRef, useState } from 'react';
import {
    Activity,
    ArrowRight,
    ArrowUp,
    AtSign,
    Bell,
    Bot,
    Check,
    ChevronDown,
    ChevronRight,
    Circle,
    CircleDot,
    Ellipsis,
    ExternalLink,
    Flag,
    GitBranch,
    Hash,
    Info,
    Layers,
    Link2,
    Menu,
    MessageSquare,
    PanelRight,
    PanelRightClose,
    Plus,
    Radio,
    RefreshCw,
    Send,
    SquarePen,
    Star,
    Terminal,
    Users,
    X,
} from 'lucide-react';
import { api, Avatar, Body, choice, Data, Form, IconButton, options, required, Section } from './ui';
import { Working } from './work';

export function Mission({
    m,
    t,
    all,
    me,
    edit,
    act,
    refresh,
    selectThread,
    run,
    setRun,
    details,
    setDetails,
}: any) {
    const [threads, setThreads] = useState(false),
        [external, setExternal] = useState(false),
        [allExternal, setAllExternal] = useState(false);
    const [body, setBody] = useState(''),
        [picked, setPicked] = useState<any[]>([]),
        [refs, setRefs] = useState<any[]>([]),
        [confirm, setConfirm] = useState(false),
        [error, setError] = useState(''),
        [busy, setBusy] = useState(false),
        [menu, setMenu] = useState('');
    const [members, setMembers] = useState<any[]>([]);
    const [membershipNotice, setMembershipNotice] = useState('');
    const input = useRef<HTMLTextAreaElement>(null),
        scroll = useRef<HTMLDivElement>(null);
    const tid = t.thread.id;
    useEffect(() => {
        setBody('');
        setPicked([]);
        setRefs([]);
        setConfirm(false);
        setError('');
        setMenu('');
        setThreads(false);
        setExternal(false);
    }, [tid]);
    useEffect(() => {
        let keep = true;
        api('member.list', { mission: m.id })
            .then((r) => {
                if (keep) setMembers(r);
            })
            .catch((e) => setError(e.message));
        return () => {
            keep = false;
        };
    }, [m.id, t.messages.length, all]);
    useEffect(() => {
        if (scroll.current) scroll.current.scrollTop = scroll.current.scrollHeight;
    }, [tid, t.messages.length]);
    const agents = all.agents.filter((a: any) => a.introduce || members.some((m) => m.id === a.id));
    const mentionable = [...agents.map((x: any) => ({ ...x, kind: 'agent' })), ...all.people];
    const explicit = mentionable.filter((x) =>
        body.match(
            new RegExp(
                '(?:^|\\s)@' + x.name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&') + '(?=$|[\\s,.!?;])',
                'i',
            ),
        ),
    );
    const addressed = Array.from(
        new Map(
            [...explicit, ...picked.filter((x) => body.includes('@' + x.name))].map((x) => [x.id, x]),
        ).values(),
    );
    const links = refs.filter((x) => body.includes(x.display));
    const active = t.runs.filter((r: any) =>
        ['working', 'queued', 'interrupted', 'stopping', 'continuing'].includes(r.status),
    );
    const observed = useRef('');
    useEffect(() => {
        const latest = active[0]?.id;
        if (latest && latest !== observed.current && window.matchMedia('(min-width:761px)').matches) {
            observed.current = latest;
            if (!run) {
                setRun(latest);
                setDetails(false);
            }
        }
    }, [active[0]?.id, tid]);
    const single =
        t.participants.length === 2 &&
        t.participants.some((p: any) => p.id === me.id) &&
        t.participants.some((p: any) => p.id.startsWith('a_'));
    const blocked = single && active.length > 0 && !addressed.length;
    const fresh = addressed.filter((p: any) => !members.some((x) => x.id === p.id));
    const needsConfirm = fresh.length > 0 && !confirm;
    const suggest = body.match(/(?:^|\s)([@#])([^\n@#]*)$/);
    const addedAgent =
        addressed.some((x: any) => x.kind === 'agent' && !t.participants.some((p: any) => p.id === x.id)) &&
        t.participants.some((p: any) => p.id.startsWith('a_'));
    const [linkedThreads, setLinkedThreads] = useState<any[]>([]);
    const linkedMission = suggest?.[1] === '#' && suggest[2].includes(':') ? suggest[2].split(':')[0] : '';
    useEffect(() => {
        let current = true;
        setLinkedThreads([]);
        if (
            linkedMission &&
            linkedMission !== m.id &&
            all.missions.some((x: any) => x.id === linkedMission)
        ) {
            api('thread.list', { mission: linkedMission })
                .then((result) => {
                    if (current) setLinkedThreads(result);
                })
                .catch(() => {});
        }
        return () => {
            current = false;
        };
    }, [linkedMission, m.id]);
    const candidates =
        suggest?.[1] === '@'
            ? mentionable.filter((x) => x.name.toLowerCase().includes(suggest[2].toLowerCase())).slice(0, 6)
            : suggest?.[1] === '#'
              ? [
                    ...all.missions.map((x: any) => ({ ...x, display: '#' + x.id, name: x.title })),
                    ...[...m.threads, ...linkedThreads]
                        .filter((x: any) => x.kind !== 'working')
                        .map((x: any) => ({
                            ...x,
                            display: '#' + x.mission + ':' + x.title,
                            name: x.title,
                        })),
                ]
                    .filter((x) =>
                        (x.display + ' ' + x.name).toLowerCase().includes(suggest[2].toLowerCase()),
                    )
                    .slice(0, 6)
              : [];
    async function send() {
        if (!body.trim() || busy || blocked || needsConfirm) return;
        setError('');
        setBusy(true);
        try {
            const r = await api('post', {
                thread: tid,
                body,
                mentions: addressed.map((x: any) => x.id),
                refs: links.map((x) => x.display),
                confirm,
                key: crypto.randomUUID(),
            });
            setBody('');
            setPicked([]);
            setRefs([]);
            setConfirm(false);
            if (r.runs?.length && window.matchMedia('(min-width:761px)').matches) {
                setRun(r.runs[0]);
                setDetails(false);
            }
            refresh();
        } catch (e: any) {
            setError(e.message);
        } finally {
            setBusy(false);
        }
    }
    const showRun = (rid: string) => {
        setRun(rid);
        setDetails(false);
    };
    const openThread = (id: string) => {
        setThreads(false);
        selectThread(id);
    };
    const ext = m.threads.filter((x: any) => x.kind === 'external');
    const takeOver = (repo: any) =>
        edit({
            title: 'Take over ' + (repo.read_only ? repo.remote : repo.name),
            op: 'repo.takeover',
            initial: { mission: m.id, name: repo.name, remote: repo.remote, read_only: repo.read_only },
            fields: [{ key: 'connection', label: 'Git connection', type: 'git-access' }],
            submit: 'Use my Git access',
        });
    const readOnlyRepos = m.read_only_repos || [];
    const attach = (remote?: string) =>
        edit({
            title: 'Attach read-only repository',
            op: 'repo.attach',
            initial: { mission: m.id, remote },
            fields: [
                required('remote', 'Repository URL'),
                { key: 'connection', label: 'Git connection', type: 'git-access' },
            ],
            submit: 'Add to mission (read-only)',
        });
    const conflicts = m.repos.filter(
        (r: any) =>
            !r.needs_transfer &&
            [
                'diverged',
                'local edits',
                'unresolved merge',
                'error',
                'unpublished',
                'different branch',
            ].includes(r.status.state),
    );
    const wip = (r: any, fork = false) =>
        edit({
            title: fork ? 'Commit to a WIP branch' : 'Commit WIP',
            op: 'repo.wip',
            initial: { mission: m.id, name: r.name, thread: tid, fork },
            fields: [
                {
                    key: 'summary',
                    label: 'Commit summary',
                    placeholder: 'Generate from recent conversation and changes',
                },
            ],
            submit: fork ? 'Commit and push branch' : 'Commit changes',
        });
    const newThread = () =>
        edit({
            title: 'New thread',
            op: 'thread.create',
            initial: { mission: m.id },
            fields: [required('title', 'Thread title')],
            submit: 'Create thread',
        });
    return (
        <div className="mission-layout">
            <section className="conversation">
                <header className="mission-head">
                    <div className="mission-tools">
                        <span className="mission-key">
                            <Flag size={14} />
                            {m.id.toUpperCase()}
                        </span>
                        <div className="button-wrap">
                            <IconButton
                                title="Mission details"
                                onClick={() => {
                                    setRun('');
                                    setDetails(!details);
                                }}
                            >
                                <PanelRight />
                            </IconButton>
                            <IconButton
                                title="Edit mission"
                                onClick={() =>
                                    edit({
                                        title: 'Edit mission',
                                        op: 'mission.update',
                                        initial: {
                                            mission: m.id,
                                            title: m.title,
                                            description: m.description,
                                        },
                                        fields: [
                                            required('title', 'Mission title'),
                                            { key: 'description', label: 'Description', type: 'area' },
                                        ],
                                    })
                                }
                            >
                                <Ellipsis />
                            </IconButton>
                        </div>
                    </div>
                    <h1>{m.title}</h1>
                    {m.description && <p>{m.description}</p>}
                    <div className="mission-meta">
                        <label className="status-picker">
                            <i
                                className={
                                    'status-dot ' +
                                    (all.statuses.find((s: any) => s.id === m.status)?.color || 'slate')
                                }
                            />
                            <select
                                aria-label="Mission status"
                                value={m.status}
                                onChange={(e) =>
                                    act('mission.update', { mission: m.id, status: e.target.value })
                                }
                            >
                                {all.statuses.map((s: any) => (
                                    <option key={s.id} value={s.id}>
                                        {s.name}
                                    </option>
                                ))}
                            </select>
                        </label>
                        <button
                            onClick={() => {
                                setDetails(true);
                                setRun('');
                            }}
                        >
                            <Users />
                            {members.length} {members.length === 1 ? 'member' : 'members'}
                        </button>
                        <span className="dot">·</span>
                        <button
                            onClick={() => {
                                setDetails(true);
                                setRun('');
                            }}
                        >
                            <GitBranch />
                            {m.repos.length} {m.repos.length === 1 ? 'repository' : 'repositories'}
                        </button>
                    </div>
                </header>
                <div className="thread-bar">
                    <div className="popover-anchor">
                        <button
                            className="tab active"
                            aria-label={t.thread.title}
                            aria-description={t.thread.kind === 'main' ? 'Default thread' : undefined}
                            onClick={() => {
                                setThreads(!threads);
                                setExternal(false);
                            }}
                        >
                            <MessageSquare />
                            {t.thread.title}
                            {t.thread.kind === 'main' && (
                                <Star className="default-star">
                                    <title>Default thread</title>
                                </Star>
                            )}
                            <ChevronDown size={13} />
                        </button>
                        {threads && (
                            <div className="popover">
                                {m.threads
                                    .filter((x: any) => ['main', 'hosted'].includes(x.kind))
                                    .map((x: any) => (
                                        <button key={x.id} onClick={() => openThread(x.id)}>
                                            <MessageSquare />
                                            {x.title}
                                            {x.kind === 'main' && (
                                                <Star className="default-star">
                                                    <title>Default thread</title>
                                                </Star>
                                            )}
                                            {x.id === tid && <Check />}
                                        </button>
                                    ))}
                                <button onClick={newThread}>
                                    <Plus />
                                    New thread
                                </button>
                                <button
                                    onClick={() =>
                                        edit({
                                            title: 'Rename thread',
                                            op: 'thread.rename',
                                            initial: { thread: tid, title: t.thread.title },
                                            fields: [required('title', 'Title')],
                                        })
                                    }
                                >
                                    <SquarePen />
                                    Rename current thread
                                </button>
                            </div>
                        )}
                    </div>
                    <div className="popover-anchor">
                        <button
                            className="tab"
                            onClick={() => {
                                setExternal(!external);
                                setThreads(false);
                            }}
                        >
                            <Terminal />
                            External sessions <span className="count">{ext.length}</span>
                        </button>
                        {external && (
                            <div className="popover">
                                <strong>External sessions</strong>
                                {!ext.length && <p>Open this mission locally to capture a session.</p>}
                                {ext.slice(0, allExternal ? undefined : 5).map((x: any) => (
                                    <button
                                        key={x.id}
                                        onClick={() => {
                                            openThread(x.id);
                                            setExternal(false);
                                        }}
                                    >
                                        <Terminal />
                                        {x.title}
                                    </button>
                                ))}
                                {ext.length > 5 && (
                                    <button onClick={() => setAllExternal(!allExternal)}>
                                        {allExternal ? 'Show recent' : 'See all'}
                                    </button>
                                )}
                            </div>
                        )}
                    </div>
                    <div className="spacer" />
                    <IconButton
                        title="Thread subscriptions"
                        onClick={() =>
                            edit({
                                title: 'Thread notifications',
                                op: 'thread.subscribe',
                                initial: { thread: tid, mode: t.subscription },
                                fields: [
                                    choice('mode', 'Notify me about', [
                                        { value: 'participating', label: 'Mentions and my agent runs' },
                                        { value: 'all', label: 'All messages' },
                                        { value: 'muted', label: 'Muted thread activity' },
                                    ]),
                                ],
                            })
                        }
                    >
                        <Bell />
                    </IconButton>
                </div>
                <div className="timeline" ref={scroll}>
                    {!t.messages.length ? (
                        <div className="empty thread-empty">
                            <span className="empty-icon">
                                <MessageSquare />
                            </span>
                            <h3>Start the conversation</h3>
                            <p>Send a message or mention an agent in this mission.</p>
                        </div>
                    ) : (
                        t.messages.map((msg: any, i: number) => {
                            const day = new Date(msg.created).toLocaleDateString(undefined, {
                                month: 'short',
                                day: 'numeric',
                                year: 'numeric',
                            });
                            const prev = t.messages[i - 1];
                            return (
                                <React.Fragment key={msg.id}>
                                    {(!prev ||
                                        new Date(prev.created).toDateString() !==
                                            new Date(msg.created).toDateString()) && (
                                        <div className="date-divider">
                                            <span>{day}</span>
                                        </div>
                                    )}
                                    <article className="message" id={'msg-' + msg.id}>
                                        <Avatar name={msg.author} agent={msg.source === 'agent'} />
                                        <div className="message-main">
                                            <div className="byline">
                                                <b>{msg.author}</b>
                                                {msg.source !== 'rally' && (
                                                    <span className="source">{msg.source}</span>
                                                )}
                                                <time dateTime={msg.created}>
                                                    {new Date(msg.created).toLocaleTimeString([], {
                                                        hour: 'numeric',
                                                        minute: '2-digit',
                                                    })}
                                                </time>
                                            </div>
                                            <Body text={msg.body} links={msg.links} />
                                            {t.runs
                                                .filter((r: any) => r.trigger === msg.id)
                                                .map((r: any) => (
                                                    <button
                                                        key={r.id}
                                                        className="activity-link"
                                                        onClick={() => showRun(r.id)}
                                                    >
                                                        <Activity />
                                                        {r.name}{' '}
                                                        {active.includes(r)
                                                            ? 'is working'
                                                            : r.status === 'done'
                                                              ? 'finished'
                                                              : r.status}
                                                        <ChevronRight />
                                                    </button>
                                                ))}
                                        </div>
                                    </article>
                                </React.Fragment>
                            );
                        })
                    )}
                </div>
                {t.thread.kind !== 'external' && (
                    <div className="composer-wrap">
                        {active
                            .filter((r: any) => !t.messages.some((msg: any) => msg.id === r.trigger))
                            .map((r: any) => (
                                <button key={r.id} onClick={() => setRun(r.id)}>
                                    <Activity />
                                    {r.agent_name || r.name || 'Agent'} is working
                                </button>
                            ))}
                        {(m.git_requests || []).map((remote: string) => (
                            <div className="notice git-request" key={remote}>
                                <span>
                                    Git read access requested for <b>{remote}</b>{' '}
                                    <TransportLabel url={remote} />
                                </span>
                                <details className="git-help">
                                    <summary
                                        title={gitRequestHelp}
                                        aria-label="About this repository request"
                                    >
                                        <Info />
                                    </summary>
                                    <span>{gitRequestHelp}</span>
                                </details>
                                <button onClick={() => attach(remote)}>Add to mission (read-only)</button>
                                <button onClick={() => act('repo.dismiss', { mission: m.id, remote })}>
                                    Dismiss
                                </button>
                            </div>
                        ))}
                        {[...m.repos, ...readOnlyRepos]
                            .filter((r: any) => r.needs_transfer)
                            .map((r: any) => (
                                <div className="notice warning" key={r.name}>
                                    <GitBranch />
                                    <span>
                                        Hosted Git access paused for <b>{r.read_only ? r.remote : r.name}</b>.
                                    </span>
                                    <button onClick={() => takeOver(r)}>Take over</button>
                                </div>
                            ))}
                        {conflicts.map((r: any) => (
                            <div className="notice warning" key={r.name}>
                                <GitBranch />
                                <span>
                                    <b>{r.name}</b> · {r.status.state}
                                </span>
                                <button onClick={() => act('repo.sync', { mission: m.id })}>Refresh</button>
                                {(r.status.dirty || r.status.state === 'diverged') &&
                                    r.status.state !== 'unresolved merge' && (
                                        <button onClick={() => wip(r, true)}>Commit to WIP branch</button>
                                    )}
                            </div>
                        ))}
                        {fresh.length > 0 && (
                            <div className="notice">
                                <Users />
                                <span>
                                    Give {fresh.map((x: any) => x.name).join(', ')} access to the entire
                                    mission history?
                                    {fresh.some((x: any) => x.private) &&
                                        ' Everyone in this mission will also be able to use the invited private agent and any secrets the agent has.'}
                                </span>
                                <button onClick={() => setConfirm(true)}>
                                    {confirm ? 'Approved' : 'Approve'}
                                </button>
                            </div>
                        )}
                        {addedAgent && (
                            <div className="notice">
                                This thread will include multiple agents. You can also start a separate
                                thread.
                            </div>
                        )}
                        {links.length > 0 && (
                            <div className="notice">
                                Referenced context can be shared with all current and future mission members.
                            </div>
                        )}
                        {blocked && (
                            <div className="notice">
                                Your agent is working. Mention it to steer or{' '}
                                <button onClick={() => showRun(active[0].id)}>open activity</button>.
                            </div>
                        )}
                        {addressed.some((p: any) => active.some((r: any) => r.agent === p.id)) && (
                            <div className="notice">This message will steer the active run.</div>
                        )}
                        {error && (
                            <p role="alert" className="error">
                                {error}
                            </p>
                        )}
                        <div className="composer">
                            {candidates.length > 0 && (
                                <div className="suggest">
                                    {candidates.map((x: any) => (
                                        <button
                                            key={x.id}
                                            onClick={() => {
                                                const str = x.display || '@' + x.name;
                                                setBody(
                                                    body.slice(0, body.lastIndexOf(suggest![1])) + str + ' ',
                                                );
                                                if (x.display) setRefs([...refs, x]);
                                                else setPicked([...picked, x]);
                                                setConfirm(false);
                                                input.current?.focus();
                                            }}
                                        >
                                            {x.display ? <Hash /> : <Avatar small name={x.name} />}
                                            <b>{x.display || x.name}</b>
                                            {x.display && <span>{x.name}</span>}
                                        </button>
                                    ))}
                                </div>
                            )}
                            <textarea
                                ref={input}
                                aria-label="Message"
                                value={body}
                                onChange={(e) => {
                                    setBody(e.target.value);
                                    setConfirm(false);
                                }}
                                onKeyDown={(e) => {
                                    if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
                                        e.preventDefault();
                                        send();
                                    }
                                }}
                                placeholder={'Message ' + t.thread.title + '…'}
                            />
                            <div className="composer-actions">
                                <IconButton
                                    title="Mention a teammate or agent"
                                    onClick={() => {
                                        setBody(body + (body && !body.endsWith(' ') ? ' ' : '') + '@');
                                        input.current?.focus();
                                    }}
                                >
                                    <AtSign />
                                </IconButton>
                                <IconButton
                                    title="Link a mission"
                                    onClick={() => {
                                        setBody(body + (body && !body.endsWith(' ') ? ' ' : '') + '#');
                                        input.current?.focus();
                                    }}
                                >
                                    <Hash />
                                </IconButton>
                                <span className="spacer" />
                                <button
                                    className="send"
                                    title="Send message"
                                    aria-label="Send message"
                                    disabled={busy || !body.trim() || blocked || needsConfirm}
                                    onClick={send}
                                >
                                    <ArrowUp />
                                </button>
                            </div>
                        </div>
                    </div>
                )}
            </section>
            {run ? (
                <Working key={run} id={run} close={() => setRun('')} refresh={refresh} />
            ) : (
                details && (
                    <aside className="side-pane">
                        <header className="pane-head">
                            <Layers />
                            <b>Mission details</b>
                            <IconButton title="Close details" onClick={() => setDetails(false)}>
                                <PanelRightClose />
                            </IconButton>
                        </header>
                        <div className="pane-content">
                            <Section
                                title="Repositories"
                                action={
                                    <IconButton
                                        title="Add repository"
                                        onClick={() =>
                                            edit({
                                                title: 'Track a repository',
                                                op: 'repo.add',
                                                initial: { mission: m.id, branch: 'main' },
                                                fields: [
                                                    required('name', 'Checkout directory'),
                                                    required('remote', 'Git remote'),
                                                    {
                                                        key: 'connection',
                                                        label: 'Git connection',
                                                        type: 'git-access',
                                                    },
                                                    required('branch', 'Branch'),
                                                ],
                                            })
                                        }
                                    >
                                        <Plus />
                                    </IconButton>
                                }
                            >
                                {!m.repos.length && !readOnlyRepos.length && (
                                    <p className="muted">No repositories attached.</p>
                                )}
                                {m.repos.map((r: any) => (
                                    <div className="repo" key={r.name}>
                                        <strong>
                                            <GitBranch />
                                            {r.name}
                                        </strong>
                                        <small>
                                            {r.recovery || r.branch} ·{' '}
                                            {r.needs_transfer
                                                ? 'Needs transfer'
                                                : r.status.state || 'Preparing'}
                                            {r.contributor_name && ' · Contributed by ' + r.contributor_name}{' '}
                                            <TransportLabel url={r.upstream} />
                                        </small>
                                        {r.status.error && <p className="error">{r.status.error}</p>}
                                        <div className="button-wrap">
                                            <button onClick={() => act('repo.sync', { mission: m.id })}>
                                                Refresh
                                            </button>
                                            <button onClick={() => wip(r)}>Commit WIP</button>
                                            <button disabled={r.needs_transfer} onClick={() => wip(r, true)}>
                                                WIP branch
                                            </button>
                                            <button
                                                disabled={r.needs_transfer || !r.connection}
                                                onClick={() =>
                                                    act('repo.push', { mission: m.id, name: r.name })
                                                }
                                            >
                                                Push
                                            </button>
                                            <button onClick={() => takeOver(r)}>Take over</button>
                                            {!r.needs_transfer && me.id === r.contributor && (
                                                <button
                                                    onClick={() =>
                                                        act('repo.withdraw', { mission: m.id, name: r.name })
                                                    }
                                                >
                                                    Withdraw my access
                                                </button>
                                            )}
                                            {r.recovery && !r.needs_transfer && (
                                                <button
                                                    onClick={() =>
                                                        act('repo.return', { mission: m.id, name: r.name })
                                                    }
                                                >
                                                    Return to {r.branch}
                                                </button>
                                            )}
                                        </div>
                                    </div>
                                ))}
                            </Section>
                            <Section
                                title="Read-only repositories"
                                action={
                                    <IconButton title="Attach read-only repository" onClick={() => attach()}>
                                        <Plus />
                                    </IconButton>
                                }
                            >
                                {readOnlyRepos.map((r: any) => (
                                    <div className="repo" key={r.name}>
                                        <strong>{r.remote}</strong>
                                        <small>
                                            {r.needs_transfer ? 'Needs transfer' : 'Read-only'}
                                            {r.contributor_name &&
                                                ' · Contributed by ' + r.contributor_name}{' '}
                                            <TransportLabel url={r.upstream} />
                                        </small>
                                        <div className="button-wrap">
                                            <button onClick={() => takeOver(r)}>Take over</button>
                                            {!r.needs_transfer && me.id === r.contributor && (
                                                <button
                                                    onClick={() =>
                                                        act('repo.withdraw', { mission: m.id, name: r.name })
                                                    }
                                                >
                                                    Withdraw my access
                                                </button>
                                            )}
                                            <button
                                                onClick={() =>
                                                    act('repo.remove', { mission: m.id, name: r.name })
                                                }
                                            >
                                                Detach
                                            </button>
                                        </div>
                                    </div>
                                ))}
                            </Section>
                            <Section
                                title="Members"
                                action={
                                    <IconButton
                                        title="Add member"
                                        onClick={() =>
                                            edit({
                                                title: 'Grant mission access',
                                                op: 'member.add',
                                                initial: { mission: m.id },
                                                fields: [
                                                    choice(
                                                        'principal',
                                                        'Person, group, or agent',
                                                        options([
                                                            ...all.people,
                                                            ...all.groups,
                                                            ...agents.filter((a: any) => a.introduce),
                                                        ]),
                                                    ),
                                                    {
                                                        key: 'confirm',
                                                        type: 'checkbox',
                                                        when: 'principal',
                                                        values: agents
                                                            .filter((a: any) => a.private)
                                                            .map((a: any) => a.id),
                                                        required: true,
                                                        label: 'Give all mission members use of this private agent and any secrets the agent has',
                                                    },
                                                ],
                                                submit: 'Grant access to history',
                                            })
                                        }
                                    >
                                        <Plus />
                                    </IconButton>
                                }
                            >
                                {membershipNotice && <p role="status">{membershipNotice}</p>}
                                {members.map((p) => (
                                    <div className="person" key={p.id}>
                                        <Avatar name={p.name} small agent={p.kind === 'agent'} />
                                        <span>{p.name}</span>
                                        <small>{p.kind}</small>
                                        {p.kind !== 'agent' && (
                                            <button
                                                onClick={async () => {
                                                    const result = await act('member.remove', {
                                                        mission: m.id,
                                                        principal: p.id,
                                                    });
                                                    if (result)
                                                        setMembershipNotice(
                                                            p.name +
                                                                ' removed.' +
                                                                (result.remaining?.length
                                                                    ? ' Access continues through ' +
                                                                      result.remaining.join(', ') +
                                                                      '.'
                                                                    : '') +
                                                                (result.retained?.length
                                                                    ? ' Other grants keep access for ' +
                                                                      result.retained
                                                                          .map((person: any) => person.name)
                                                                          .join(', ') +
                                                                      '.'
                                                                    : ''),
                                                        );
                                                }}
                                            >
                                                Remove
                                            </button>
                                        )}
                                    </div>
                                ))}
                            </Section>
                            <SecretsPanel target={m.id} />
                            <Section title="Local workspace">
                                <code>rally open '#{m.id}'</code>
                            </Section>
                        </div>
                    </aside>
                )
            )}
        </div>
    );
}
