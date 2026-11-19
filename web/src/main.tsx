import React, { useCallback, useEffect, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import {
    Activity,
    ArrowLeft,
    ArrowRight,
    AtSign,
    Bell,
    Bot,
    Box,
    Check,
    ChevronDown,
    ChevronRight,
    CircleDot,
    Command,
    Flag,
    Folder,
    GitBranch,
    Hash,
    Inbox,
    Layers,
    Library,
    Menu,
    MessageSquare,
    PanelLeftClose,
    PanelLeftOpen,
    Plus,
    Search,
    Settings,
    SquarePen,
    Terminal,
    Webhook,
    X,
} from 'lucide-react';
import { api, Auth, Avatar, Brand, choice, Editor, Form, IconButton, Modal, options, required } from './ui';
import { MissionNav } from './mission-nav';
import { Mission } from './mission';
import { Manage } from './manage';
import './styles.css';

const empty = {
    topics: [],
    statuses: [],
    missions: [],
    agents: [],
    people: [],
    skills: [],
    routines: [],
    groups: [],
    inbox: [],
};
function App() {
    const [me, setMe] = useState<any>(undefined),
        [all, setAll] = useState<any>(empty),
        [page, setPage] = useState('Missions'),
        [mid, setMid] = useState(''),
        [tid, setTid] = useState(''),
        [m, setMission] = useState<any>(null),
        [t, setThread] = useState<any>(null);
    const [rev, setRev] = useState(0),
        [tick, setTick] = useState(0),
        [error, setError] = useState(''),
        [form, setForm] = useState<Form | null>(null),
        [sidebar, setSidebar] = useState(false),
        [collapsed, setCollapsed] = useState<string[]>([]),
        [search, setSearch] = useState(false),
        [q, setQ] = useState(''),
        [hits, setHits] = useState<any[]>([]),
        [scope, setScope] = useState(''),
        [searching, setSearching] = useState(false);
    const [run, setRun] = useState(''),
        [details, setDetails] = useState(false);
    const refresh = useCallback(() => {
        setRev((n) => n + 1);
        setTick((n) => n + 1);
    }, []);
    const auth = () =>
        api('whoami')
            .then(setMe)
            .catch(() => setMe(null));
    useEffect(() => {
        auth();
        if ('serviceWorker' in navigator) navigator.serviceWorker.register('/sw.js').catch(() => {});
    }, []);
    useEffect(() => {
        if (!me) return;
        let live = true;
        const entries = [
            ['topics', 'topic.list'],
            ['statuses', 'status.list'],
            ['missions', 'mission.list'],
            ['agents', 'agent.list'],
            ['people', 'people.list'],
            ['skills', 'skill.list'],
            ['routines', 'routine.list'],
            ['groups', 'group.list'],
            ['inbox', 'inbox.list'],
        ];
        Promise.all(entries.map(async ([key, op]) => [key, await api(op)] as const))
            .then((pairs) => {
                if (live) setAll(Object.fromEntries(pairs));
            })
            .catch((e) => setError(e.message));
        return () => {
            live = false;
        };
    }, [me, rev]);
    useEffect(() => {
        if (!me) return;
        const es = new EventSource('/events');
        let timer: any;
        es.onmessage = () => {
            if (!timer)
                timer = setTimeout(() => {
                    setTick((n) => n + 1);
                    api('inbox.list')
                        .then((inbox) => setAll((d: any) => ({ ...d, inbox })))
                        .catch(() => {});
                    timer = null;
                }, 180);
        };
        return () => {
            es.close();
            clearTimeout(timer);
        };
    }, [me]);
    useEffect(() => {
        function route() {
            const address = location.hash;
            setForm(history.state?.rallyForm || null);
            const [kind, key, _, rid] = location.hash
                .slice(1)
                .split('?')[0]
                .split('/')
                .map(decodeURIComponent);
            setSidebar(false);
            setDetails(false);
            setRun(rid || '');
            if (kind === 'thread' && key) {
                setPage('Mission');
                setTid(key);
                api('thread.show', { thread: key })
                    .then((x) => {
                        if (location.hash === address) setMid(x.thread.mission);
                    })
                    .catch((e) => setError(e.message));
            } else if (kind === 'mission' && key) {
                setPage('Mission');
                setMid(key);
                setTid('');
            } else {
                setPage(
                    ['Inbox', 'Agents', 'Skills', 'Routines', 'Settings'].includes(kind) ? kind : 'Missions',
                );
                setTid('');
                setMid('');
            }
        }
        if (me) route();
        window.addEventListener('hashchange', route);
        return () => window.removeEventListener('hashchange', route);
    }, [me]);
    useEffect(() => {
        if (!mid || page !== 'Mission') return;
        let current = true;
        (async () => {
            const mission = await api('mission.show', { mission: mid });
            const id = tid || mission.threads.find((t: any) => t.kind === 'main').id;
            const thread = await api('thread.show', { thread: id });
            if (current && thread.thread.mission === mid) {
                setMission(mission);
                setThread(thread);
            }
        })().catch((e) => setError(e.message));
        return () => {
            current = false;
        };
    }, [mid, tid, tick, page]);
    useEffect(() => {
        if (t?.thread?.id && page === 'Mission') {
            api('thread.seen', { thread: t.thread.id })
                .then(() => api('inbox.list'))
                .then((inbox) => setAll((d: any) => ({ ...d, inbox })))
                .catch(() => {});
        }
    }, [page, t?.thread?.id, t?.messages?.length]);
    useEffect(() => {
        function key(e: KeyboardEvent) {
            if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
                e.preventDefault();
                setSearch((s) => !s);
            }
        }
        window.addEventListener('keydown', key);
        return () => window.removeEventListener('keydown', key);
    }, []);
    useEffect(() => {
        if (!search || !q.trim()) {
            setHits([]);
            return;
        }
        let current = true;
        setSearching(true);
        const timer = setTimeout(
            () =>
                api('search', { query: q, mission: scope })
                    .then((r) => {
                        if (current) setHits(r);
                    })
                    .catch((e) => setError(e.message))
                    .finally(() => {
                        if (current) setSearching(false);
                    }),
            180,
        );
        return () => {
            current = false;
            clearTimeout(timer);
        };
    }, [search, q, scope]);
    const nav = (s: string) => {
        setSidebar(false);
        location.hash = s;
    };
    const selectMission = (id: string) => nav('mission/' + id);
    const selectThread = (id: string) => nav('thread/' + id);
    const act = async (op: string, d: any) => {
        try {
            const r = await api(op, d);
            refresh();
            return r;
        } catch (e: any) {
            setError(e.message);
        }
    };
    const openForm = (next: Form) => {
        const from = location.hash.split('?')[0] || '#' + page;
        history.pushState({ rallyForm: next }, '', from + '?form=' + encodeURIComponent(next.op!));
        setSidebar(false);
        setForm(next);
    };
    const newTopic = () =>
        openForm({
            title: 'New topic',
            op: 'topic.create',
            fields: [required('name', 'Topic name'), required('slug', 'Mission key prefix')],
            submit: 'Create topic',
        });
    const newMission = (topic?: string) => {
        if (!all.topics.length) {
            openForm({
                title: 'Create your first mission',
                fields: [
                    required('topicName', 'Topic name'),
                    required('slug', 'Mission key prefix'),
                    required('title', 'Mission title'),
                ],
                initial: { topicName: 'Engineering', slug: 'dev' },
                submit: 'Create mission',
                op: 'mission.bootstrap',
            });
            return;
        }
        openForm({
            title: 'New mission',
            op: 'mission.create',
            fields: [
                required('title', 'Mission title'),
                choice('topic', 'Topic', options(all.topics, 'slug')),
                { key: 'description', label: 'Description', type: 'area' },
            ],
            initial: { topic: topic || all.topics[0].slug },
            submit: 'Create mission',
        });
    };
    const navs = [
        ['Inbox', Inbox],
        ['Missions', Flag],
        ['Agents', Bot],
        ['Skills', Library],
        ['Routines', Webhook],
    ] as const;
    const unread = all.inbox.filter((n: any) => !n.seen).length;
    if (me === undefined)
        return (
            <main className="loading">
                <Brand />
            </main>
        );
    if (!me) return <Auth done={auth} />;
    return (
        <div className="app">
            {sidebar && (
                <button className="scrim" aria-label="Close sidebar" onClick={() => setSidebar(false)} />
            )}
            <aside className={'sidebar ' + (sidebar ? 'open' : '')}>
                <div className="workspace-brand">
                    <button className="brand" onClick={() => nav('Missions')}>
                        <Brand />
                        Rally
                    </button>
                    <IconButton title="New mission" onClick={() => newMission()}>
                        <SquarePen />
                    </IconButton>
                </div>
                <button className="search-launch" onClick={() => setSearch(true)}>
                    <Search />
                    Search <kbd>⌘ K</kbd>
                </button>
                <nav aria-label="Workspace">
                    {navs.map(([name, Icon]) => (
                        <button
                            key={name}
                            className={'nav-row ' + (page === name ? 'selected' : '')}
                            onClick={() => nav(name)}
                        >
                            <Icon />
                            {name}
                            {name === 'Inbox' && unread > 0 && <span className="count">{unread}</span>}
                        </button>
                    ))}
                </nav>
                <div className="nav-header">
                    <span>Topics</span>
                    <IconButton title="New topic" onClick={newTopic}>
                        <Plus />
                    </IconButton>
                </div>
                <div className="topic-list">
                    {all.topics.map((topic: any) => (
                        <div key={topic.slug}>
                            <div className="topic-head">
                                <button
                                    aria-expanded={!collapsed.includes(topic.slug)}
                                    onClick={() =>
                                        setCollapsed(
                                            collapsed.includes(topic.slug)
                                                ? collapsed.filter((x) => x !== topic.slug)
                                                : [...collapsed, topic.slug],
                                        )
                                    }
                                >
                                    {collapsed.includes(topic.slug) ? <ChevronRight /> : <ChevronDown />}
                                    <span className="topic-square" />
                                    <span className="topic-name">{topic.name}</span>
                                    <code className="topic-key" title={topic.slug.toUpperCase()}>
                                        ({topic.slug.toUpperCase()})
                                    </code>
                                    <small className="topic-count" title="Missions">
                                        {all.missions.filter((row: any) => row.topic === topic.slug).length}
                                    </small>
                                </button>
                                <IconButton
                                    title={'New mission in ' + topic.name}
                                    onClick={() => newMission(topic.slug)}
                                >
                                    <Plus />
                                </IconButton>
                            </div>
                            {!collapsed.includes(topic.slug) &&
                                all.missions
                                    .filter((m: any) => m.topic === topic.slug)
                                    .map((row: any) => (
                                        <MissionNav
                                            key={row.id}
                                            mission={row}
                                            active={page === 'Mission' && mid === row.id}
                                            thread={t?.thread?.id}
                                            revision={rev}
                                            focusedThreads={
                                                page === 'Mission' && m?.id === row.id ? m.threads : undefined
                                            }
                                            openMission={selectMission}
                                            openThread={selectThread}
                                        />
                                    ))}
                        </div>
                    ))}
                </div>
                <button className="profile" onClick={() => nav('Settings')}>
                    <Avatar small name={me.name} />
                    <b>{me.name}</b>
                    <Settings />
                </button>
            </aside>
            <main className="main">
                <div className="crumb">
                    <IconButton title="Open navigation" onClick={() => setSidebar(true)}>
                        <Menu />
                    </IconButton>
                    <span>Workspace</span>
                    <ChevronRight />
                    <button onClick={() => nav(page === 'Mission' ? 'Missions' : page)}>
                        {page === 'Mission' ? 'Missions' : page}
                    </button>
                    {page === 'Mission' && (
                        <>
                            <ChevronRight />
                            <em>{mid.toUpperCase()}</em>
                        </>
                    )}
                    <span className="spacer" />
                    <IconButton title="Search missions" onClick={() => setSearch(true)}>
                        <Search />
                    </IconButton>
                </div>
                {error && (
                    <div className="global-error" role="alert">
                        {error}
                        <IconButton title="Dismiss error" onClick={() => setError('')}>
                            <X />
                        </IconButton>
                    </div>
                )}
                <div className="page-content" hidden={!!form}>
                    {page === 'Mission' && m?.id === mid && t?.thread?.mission === mid ? (
                        <Mission
                            key={mid}
                            m={m}
                            t={t}
                            all={all}
                            me={me}
                            edit={openForm}
                            act={act}
                            refresh={refresh}
                            selectThread={selectThread}
                            run={run}
                            setRun={setRun}
                            details={details}
                            setDetails={setDetails}
                        />
                    ) : page === 'Missions' ? (
                        <section className="page">
                            <div className="page-title">
                                <h1>Missions</h1>
                                <button className="primary" onClick={() => newMission()}>
                                    <Plus />
                                    New mission
                                </button>
                            </div>
                            {!all.missions.length && (
                                <div className="empty">
                                    <span className="empty-icon">
                                        <Flag />
                                    </span>
                                    <h3>No missions yet</h3>
                                    <button onClick={() => newMission()}>Create your first mission</button>
                                </div>
                            )}
                            {all.topics.map((top: any) => {
                                const missions = all.missions.filter((m: any) => m.topic === top.slug);
                                return (
                                    <section className="mission-group" key={top.slug}>
                                        <header>
                                            <span className="topic-square" />
                                            <b>{top.name}</b>
                                            <span className="count">{missions.length}</span>
                                            <div className="spacer" />
                                            <IconButton
                                                title={'Add mission to ' + top.name}
                                                onClick={() => newMission(top.slug)}
                                            >
                                                <Plus />
                                            </IconButton>
                                        </header>
                                        {missions.map((m: any) => (
                                            <button
                                                className="mission-record"
                                                key={m.id}
                                                onClick={() => selectMission(m.id)}
                                            >
                                                <CircleDot />
                                                <code>{m.id.toUpperCase()}</code>
                                                <span>{m.title}</span>
                                                <small className="record-status">
                                                    {all.statuses.find((s: any) => s.id === m.status)?.name}
                                                </small>
                                                <ChevronRight />
                                            </button>
                                        ))}
                                    </section>
                                );
                            })}
                        </section>
                    ) : page === 'Inbox' ? (
                        <section className="page">
                            <div className="page-title">
                                <h1>Inbox</h1>
                                <button onClick={() => act('inbox.read', {})}>
                                    <Check />
                                    Mark all read
                                </button>
                            </div>
                            {!all.inbox.length && (
                                <div className="empty">
                                    <span className="empty-icon">
                                        <Inbox />
                                    </span>
                                    <h3>You’re all caught up</h3>
                                    <p>Mentions, agent questions and results appear here.</p>
                                </div>
                            )}
                            {all.inbox.map((n: any) => (
                                <button
                                    className={'inbox-row ' + (n.seen ? '' : 'unread')}
                                    key={n.id}
                                    onClick={() => {
                                        act('inbox.read', { id: String(n.id) });
                                        nav('thread/' + n.thread + (n.run ? '/run/' + n.run : ''));
                                    }}
                                >
                                    <Bell />
                                    <span>
                                        <b>{n.summary}</b>
                                        <small>
                                            #{n.mission} · {n.title}
                                        </small>
                                    </span>
                                    <time>{new Date(n.created).toLocaleDateString()}</time>
                                </button>
                            ))}
                        </section>
                    ) : page === 'Mission' ? (
                        <div className="page muted">Loading mission…</div>
                    ) : (
                        <Manage
                            page={page}
                            data={all}
                            refresh={refresh}
                            user={me}
                            edit={openForm}
                            act={act}
                            revision={rev}
                            exit={async () => {
                                const registration = await navigator.serviceWorker?.getRegistration();
                                const device = await registration?.pushManager?.getSubscription();
                                if (device) {
                                    await api('device.unsubscribe', { endpoint: device.endpoint });
                                    await device.unsubscribe();
                                }
                                await api('logout', {}, true);
                                setMe(null);
                            }}
                        />
                    )}
                </div>
                {form && (
                    <Editor
                        key={form.op}
                        form={form}
                        parent={page === 'Mission' ? m?.title || 'mission' : page}
                        changed={refresh}
                        close={() => history.back()}
                        saved={(result) => {
                            if (['mission.bootstrap', 'mission.create', 'thread.create'].includes(form.op!)) {
                                const target = form.op === 'thread.create' ? 'thread' : 'mission';
                                history.replaceState(null, '', '#' + target + '/' + result.id);
                                window.dispatchEvent(new HashChangeEvent('hashchange'));
                            } else history.back();
                        }}
                    />
                )}
            </main>
            {search && (
                <Modal title="Search" wide onClose={() => setSearch(false)}>
                    <div className="search-box">
                        <Search />
                        <input
                            autoFocus
                            placeholder="Search decisions, messages, or missions…"
                            value={q}
                            onChange={(e) => setQ(e.target.value)}
                        />
                    </div>
                    <div className="scope">
                        <label>
                            Search in{' '}
                            <select value={scope} onChange={(e) => setScope(e.target.value)}>
                                <option value="">All accessible missions</option>
                                {all.missions.map((m: any) => (
                                    <option key={m.id} value={m.id}>
                                        {m.id} · {m.title}
                                    </option>
                                ))}
                            </select>
                        </label>
                    </div>
                    <div className="results">
                        {all.missions
                            .filter(
                                (m: any) =>
                                    (!scope || scope === m.id) &&
                                    q.trim() &&
                                    (m.title + ' ' + m.id).toLowerCase().includes(q.toLowerCase()),
                            )
                            .map((m: any) => (
                                <button
                                    key={m.id}
                                    onClick={() => {
                                        setSearch(false);
                                        selectMission(m.id);
                                    }}
                                >
                                    <Flag />
                                    <div>
                                        <small>{m.id}</small>
                                        <b>{m.title}</b>
                                    </div>
                                </button>
                            ))}
                        {hits.map((h) => (
                            <button
                                key={h.id}
                                onClick={() => {
                                    setSearch(false);
                                    selectThread(h.thread);
                                    setTimeout(
                                        () =>
                                            document
                                                .getElementById('msg-' + h.id)
                                                ?.scrollIntoView({ block: 'center' }),
                                        400,
                                    );
                                }}
                            >
                                <MessageSquare />
                                <div>
                                    <small>
                                        {h.reference} · {h.author}
                                    </small>
                                    <span>{h.body}</span>
                                </div>
                            </button>
                        ))}
                        {q && !hits.length && !searching && <p>No matching messages.</p>}
                    </div>
                </Modal>
            )}
        </div>
    );
}
createRoot(document.getElementById('root')!).render(<App />);
