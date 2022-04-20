# Turbo ISL

Turbo Interface Specification Language is a language for defining interfaces between
different domains (client/server, wasm host/guest). The syntax is based on
[S-expressions](https://en.wikipedia.org/wiki/S-expression) and looks like

```lisp

;; everything lives inside a module
(mod the-module
"Optional docstring"
(
  ;; define a record
  (rec a-record (:field1 int :field2 bool))

  ;; define a enum
  (enu an-enum (:variant1 :variant2))

  ;; A simple function
  (fun a-fun-function (:param1 int :param2 a-record) (:return-value string))
))

```

The chosen file extension for Turbo ISL files is `.tisl`.

## Turbo ISL Compiler

This package contains the code for a compiler from Turbo ISL to different targets. A
target is for example Rust code for WASM. To invoke the compiler, call `turbo` and the
command line interface is documented through `turbo --help`.

## What is up with the "Turbo"?

Ask [Borland](https://en.wikipedia.org/wiki/Borland_Turbo_C)! And no, there is no actual
turbine in the compiler.
