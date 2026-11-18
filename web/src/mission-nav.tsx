import React, { useEffect, useState } from 'react';
import { ChevronDown, ChevronRight, MessageSquare, Star, Terminal } from 'lucide-react';
import { api } from './ui';

type Thread = { id: string; kind: string; title: string };
export function MissionNav({
    mission,
    active,
    thread,
    revision,
    focusedThreads,
    openMission,
    openThread,
}: {
    mission: { id: string; title: string };
    active: boolean;
    thread?: string;
    revision: number;
    focusedThreads?: Thread[];
    openMission: (id: string) => void;
    openThread: (id: string) => void;
}) {
    const [expanded, setExpanded] = useState(false);
    const [loadedThreads, setThreads] = useState<Thread[]>([]);
    const [error, setError] = useState('');
    useEffect(() => {
        if (!expanded || focusedThreads) return;
        let current = true;
        api('thread.list', { mission: mission.id })
            .then((rows) => {
                if (current) {
                    setThreads(rows);
                    setError('');
                }
            })
            .catch((e) => {
                if (current) setError(e.message);
            });
        return () => {
            current = false;
        };
    }, [expanded, mission.id, revision, focusedThreads]);
    const threads = focusedThreads ?? loadedThreads;
    const groups = [
        { label: 'Threads', rows: threads.filter((t) => t.kind === 'main' || t.kind === 'hosted') },
        { label: 'External sessions', rows: threads.filter((t) => t.kind === 'external') },
    ];
    return (
        <div className="mission-tree">
            <div className={'tree-row' + (active ? ' selected' : '')}>
                <button
                    className="disclosure"
                    aria-expanded={expanded}
                    aria-label={'Show conversations in ' + mission.title}
                    onClick={() => setExpanded(!expanded)}
                >
                    {expanded ? <ChevronDown /> : <ChevronRight />}
                </button>
                <button
                    className="tree-mission"
                    title={mission.title}
                    aria-current={active ? 'page' : undefined}
                    aria-label={mission.title}
                    onClick={() => openMission(mission.id)}
                >
                    <span>{mission.title}</span>
                    <code>#{mission.id.split('-').at(-1)}</code>
                </button>
            </div>
            {expanded && (
                <div className="tree-threads">
                    {error && !focusedThreads && <small role="alert">{error}</small>}
                    {groups.map(({ label, rows }) => (
                        <details key={label} open={label === 'Threads' || undefined}>
                            <summary>
                                {label}
                                <small>{rows.length}</small>
                            </summary>
                            {rows.map((t) => (
                                <button
                                    key={t.id}
                                    className={active && thread === t.id ? 'selected' : ''}
                                    onClick={() => openThread(t.id)}
                                    title={t.title}
                                >
                                    {t.kind === 'main' ? (
                                        <Star aria-label="Default thread" />
                                    ) : t.kind === 'external' ? (
                                        <Terminal />
                                    ) : (
                                        <MessageSquare />
                                    )}
                                    <span>{t.title}</span>
                                </button>
                            ))}
                            {!rows.length && <small>No sessions yet</small>}
                        </details>
                    ))}
                </div>
            )}
        </div>
    );
}
