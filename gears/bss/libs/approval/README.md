# BSS approval units

An approval unit groups proposed business changes for one review and decision.
It records the items, reviewer snapshot, content fingerprint, quorum, generation,
and optimistic version. The submitter and every item author are excluded from
approving. Votes count only in the current generation; stale content refreshes
that generation while preserving earlier decisions for audit.

This shared library is being implemented in stages. The model, pure quorum and
separation-of-duties rules, and content fingerprint come first. The DDL, traits,
and engine follow in later tasks.

Each gear implements `Store<R>` for its approval tables and
`ApprovalSubject<R>` for each kind of business change. Both use the gear's
`toolkit_db::secure::DBRunner`; the engine runs inside the gear's transaction.
The library opens no connection. Table names are the gear's; pass a prefix to
`ddl::apply_up` when the DDL helper is implemented.
