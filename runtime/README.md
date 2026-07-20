# L++ packaged runtime objects

Phase 1 of the native-linker roadmap packages a platform runtime object at install/release time:

```text
Linux:   lpp_runtime.o
Windows: lpp_runtime.obj
```

Normal installed `lpp build` operations prefer this object over compiling `lpp_runtime.c` for each project build.

This does **not** remove the host linker yet. It removes the repeated runtime-C compilation portion of the existing build path and provides the object packaging boundary required by the future `lpp-link` executable emitter.

The source runtime remains `../lpp_runtime.c` for fallback builds and development.

## v0.1.3 documentation status

For the current supported subset and explicit feature boundaries, see
[`documentation/CURRENT_CAPABILITIES.md`](../../documentation/CURRENT_CAPABILITIES.md).

Do not use historical benchmark numbers or roadmap text as current guarantees.
