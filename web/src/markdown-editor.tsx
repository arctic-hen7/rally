import React, { useEffect, useRef, useState } from 'react';
import { EditorState } from '@codemirror/state';
import { EditorView, keymap, lineNumbers, drawSelection, highlightActiveLine } from '@codemirror/view';
import { history, historyKeymap, defaultKeymap } from '@codemirror/commands';
import { defaultHighlightStyle, syntaxHighlighting } from '@codemirror/language';
import { markdown } from '@codemirror/lang-markdown';
import { Bold, Code2, Eye, List, PenLine } from 'lucide-react';
import { Body, IconButton } from './ui';

export function MarkdownField({
    value,
    change,
    label,
}: {
    value: string;
    change: (v: string) => void;
    label: string;
}) {
    const mount = useRef<HTMLDivElement>(null);
    const editor = useRef<EditorView | null>(null);
    const latest = useRef(change);
    latest.current = change;
    const [preview, setPreview] = useState(false);
    useEffect(() => {
        if (!mount.current) return;
        const view = new EditorView({
            parent: mount.current,
            state: EditorState.create({
                doc: value,
                extensions: [
                    markdown(),
                    history(),
                    lineNumbers(),
                    drawSelection(),
                    highlightActiveLine(),
                    syntaxHighlighting(defaultHighlightStyle),
                    EditorView.lineWrapping,
                    EditorView.contentAttributes.of({
                        'aria-label': label,
                        'aria-multiline': 'true',
                        role: 'textbox',
                    }),
                    keymap.of([...defaultKeymap, ...historyKeymap]),
                    EditorView.updateListener.of((u) => {
                        if (u.docChanged) latest.current(u.state.doc.toString());
                    }),
                ],
            }),
        });
        editor.current = view;
        return () => {
            view.destroy();
            editor.current = null;
        };
    }, []);
    function format(prefix: string, suffix: string, fallback: string) {
        const view = editor.current;
        if (!view) return;
        const { from, to } = view.state.selection.main;
        const content = view.state.sliceDoc(from, to) || fallback;
        view.dispatch({
            changes: { from, to, insert: prefix + content + suffix },
            selection: { anchor: from + prefix.length, head: from + prefix.length + content.length },
        });
        view.focus();
    }
    return (
        <div className="markdown-field">
            <div className="markdown-tools">
                <button type="button" aria-pressed={!preview} onClick={() => setPreview(false)}>
                    <PenLine />
                    Write
                </button>
                <button type="button" aria-pressed={preview} onClick={() => setPreview(true)}>
                    <Eye />
                    Preview
                </button>
                <span className="spacer" />
                {!preview && (
                    <>
                        <IconButton title="Bold" onClick={() => format('**', '**', 'text')}>
                            <Bold />
                        </IconButton>
                        <IconButton title="Inline code" onClick={() => format('`', '`', 'code')}>
                            <Code2 />
                        </IconButton>
                        <IconButton title="List item" onClick={() => format('- ', '', 'item')}>
                            <List />
                        </IconButton>
                    </>
                )}
            </div>
            <div ref={mount} hidden={preview} />
            {preview && (
                <div className="markdown-preview">
                    <Body text={value} />
                </div>
            )}
        </div>
    );
}
