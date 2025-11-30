; Keywords
[
  "break"
  "case"
  "const"
  "continue"
  "default"
  "defer"
  "else"
  "enum"
  "fallthrough"
  "for"
  "func"
  "go"
  "goto"
  "if"
  "import"
  "interface"
  "map"
  "match"
  "package"
  "range"
  "return"
  "select"
  "struct"
  "type"
  "var"
] @keyword

; Operators
[
  "--"
  "-"
  "-="
  ":="
  "!"
  "!="
  "..."
  "*"
  "*="
  "/"
  "/="
  "&"
  "&&"
  "&="
  "&^"
  "&^="
  "%"
  "%="
  "^"
  "^="
  "+"
  "++"
  "+="
  "<-"
  "<"
  "<<"
  "<<="
  "<="
  "="
  "=="
  ">"
  ">="
  ">>"
  ">>="
  "|"
  "|="
  "||"
  "~"
] @operator

; Delimiters
[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
] @punctuation.bracket

[
  ","
  "."
  ":"
  ";"
] @punctuation.delimiter

; Soppo: ? for nullable types and try
"?" @operator

; Literals
(int_literal) @number
(float_literal) @number.float
(imaginary_literal) @number
(rune_literal) @character

[
  (raw_string_literal)
  (interpreted_string_literal)
] @string

(escape_sequence) @string.escape

; Nil, true, false
(nil) @constant.builtin
(true) @boolean
(false) @boolean

; Comments
(comment) @comment

; Identifiers
(identifier) @variable

(type_identifier) @type
(field_identifier) @property
(package_identifier) @namespace

; Function declarations
(function_declaration
  name: (identifier) @function)

(method_declaration
  name: (field_identifier) @function.method)

; Function calls
(call_expression
  function: (identifier) @function.call)

(call_expression
  function: (selector_expression
    field: (field_identifier) @function.method.call))

; Types
(type_spec
  name: (type_identifier) @type.definition)

(type_alias
  name: (type_identifier) @type.definition)

; Soppo: enum type keyword
"enum" @keyword

(enum_variant
  name: (identifier) @constant)

; Parameters
(parameter_declaration
  name: (identifier) @variable.parameter)

; Struct fields
(field_declaration
  name: (field_identifier) @property)

; Labels
(label_name) @label

; Imports
(import_spec
  path: (_) @string.special)

; Package clause
(package_clause
  (package_identifier) @namespace)

; Match patterns
(pattern
  (identifier) @variable)

(qualified_pattern
  qualifier: (identifier) @type
  name: (identifier) @constant)

(variant_pattern
  type: (identifier) @type)

(variant_pattern
  type: (qualified_pattern
    qualifier: (identifier) @type
    name: (identifier) @constant))

(variant_pattern
  binding: (identifier) @variable)

; Builtins
((identifier) @function.builtin
  (#any-of? @function.builtin
    "append"
    "cap"
    "close"
    "complex"
    "copy"
    "delete"
    "imag"
    "len"
    "make"
    "new"
    "panic"
    "print"
    "println"
    "real"
    "recover"))

((type_identifier) @type.builtin
  (#any-of? @type.builtin
    "any"
    "bool"
    "byte"
    "comparable"
    "complex128"
    "complex64"
    "error"
    "float32"
    "float64"
    "int"
    "int16"
    "int32"
    "int64"
    "int8"
    "rune"
    "string"
    "uint"
    "uint16"
    "uint32"
    "uint64"
    "uint8"
    "uintptr"))
