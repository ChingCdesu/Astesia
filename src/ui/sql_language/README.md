# Bundled SQL language assets

Astesia embeds the SQL highlight query from `zed-extensions/sql` commit
`fdd2a42ab9c7f63c75becc80c9c47fb0b48fe5b8` and compiles
`DerekStride/tree-sitter-sql` commit
`851e9cb257ba7c66cc8c14214a31c44d2f1e954e` through Cargo.

The query and grammar revisions must be updated together because Tree-sitter
queries reference grammar node names. They are bundled at compile time; the
application does not load the Zed extension registry or access the network at
runtime.

The copied highlight query is distributed under the adjacent MIT license.

