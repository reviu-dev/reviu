; Identifiers
(identifier) @variable

(field_expression
  (identifier) @variable.member .)

; Symbols
(quote_expression
  ":" @string.special
  [
    (identifier)
    (operator)
  ] @string.special)

; Function calls
(call_expression
  (identifier) @function)

(call_expression
  (field_expression
    (identifier) @function .))

(broadcast_call_expression
  (identifier) @function)

(broadcast_call_expression
  (field_expression
    (identifier) @function .))

; Macros
(macro_identifier) @function.special

(macro_definition
  (signature
    (call_expression
      .
      (identifier) @function.special)))

; Built-in functions
((identifier) @function.special
  (#any-of? @function.special
    "applicable" "fieldtype" "getfield" "getglobal" "invoke" "isa" "isdefined" "modifyfield!"
    "modifyglobal!" "nfields" "replacefield!" "replaceglobal!" "setfield!" "setfieldonce!"
    "setglobal!" "setglobalonce!" "swapfield!" "swapglobal!" "throw" "tuple" "typeassert" "typeof"))

; Type definitions
(type_head (_) @type)

; Type annotations
(parametrized_type_expression
  [
   (identifier) @type
   (field_expression
     (identifier) @type .)
  ]
  (curly_expression
    (_) @type))

(typed_expression
  (identifier) @type .)

(unary_typed_expression
  (identifier) @type .)

(where_expression
  (_) @type .)

(binary_expression
  (_) @type
  (operator) @operator
  (_) @type
  (#any-of? @operator "<:" ">:"))

; Built-in types
((identifier) @type.builtin
  (#any-of? @type.builtin
    "AbstractArray" "AbstractChar" "AbstractFloat" "AbstractString" "Any" "ArgumentError" "Array"
    "AssertionError" "Bool" "BoundsError" "Char" "ConcurrencyViolationError" "Cvoid" "DataType"
    "DenseArray" "DivideError" "DomainError" "ErrorException" "Exception" "Expr" "Float16" "Float32"
    "Float64" "Function" "GlobalRef" "IO" "InexactError" "InitError" "Int" "Int128" "Int16" "Int32"
    "Int64" "Int8" "Integer" "InterruptException" "LineNumberNode" "LoadError" "Method"
    "MethodError" "Module" "NTuple" "NamedTuple" "Nothing" "Number" "OutOfMemoryError"
    "OverflowError" "Pair" "Ptr" "QuoteNode" "ReadOnlyMemoryError" "Real" "Ref" "SegmentationFault"
    "Signed" "StackOverflowError" "String" "Symbol" "Task" "Tuple" "Type" "TypeError" "TypeVar"
    "UInt" "UInt128" "UInt16" "UInt32" "UInt64" "UInt8" "UndefInitializer" "UndefKeywordError"
    "UndefRefError" "UndefVarError" "Union" "UnionAll" "Unsigned" "VecElement" "WeakRef"))

; Keywords
[
  "const"
  "global"
  "local"
] @keyword

(compound_statement
  [
    "begin"
    "end"
  ] @keyword)

(quote_statement
  [
    "quote"
    "end"
  ] @keyword)

(let_statement
  [
    "let"
    "end"
  ] @keyword)

(if_statement
  [
    "if"
    "end"
  ] @keyword.control)

(elseif_clause
  "elseif" @keyword.control)

(else_clause
  "else" @keyword.control)

(ternary_expression
  [
    "?"
    ":"
  ] @keyword.control)

(try_statement
  [
    "try"
    "end"
  ] @keyword.control)

(catch_clause
  "catch" @keyword.control)

(finally_clause
  "finally" @keyword.control)

(for_statement
  [
    "for"
    "end"
  ] @keyword.control)

(for_binding
  "outer" @keyword.control)

; comprehensions
(for_clause
  "for" @keyword.control)

(if_clause
  "if" @keyword.control)

(while_statement
  [
    "while"
    "end"
  ] @keyword.control)

[
  (break_statement)
  (continue_statement)
] @keyword.control

(function_definition
  [
    "function"
    "end"
  ] @keyword)

(do_clause
  [
    "do"
    "end"
  ] @keyword)

(macro_definition
  [
    "macro"
    "end"
  ] @keyword)

(return_statement
  "return" @keyword.control)

(module_definition
  [
    "module"
    "baremodule"
    "end"
  ] @keyword)

(export_statement
  "export" @keyword)

(public_statement
  "public" @keyword)

(import_statement
  "import" @keyword)

(using_statement
  "using" @keyword)

(import_alias
  "as" @keyword)

(selected_import
  ":" @punctuation.delimiter)

(struct_definition
  [
    "mutable"
    "struct"
    "end"
  ] @keyword)

(abstract_definition
  [
    "abstract"
    "type"
    "end"
  ] @keyword)

(primitive_definition
  [
    "primitive"
    "type"
    "end"
  ] @keyword)

; Operators & Punctuation
(operator) @operator

(adjoint_expression
  "'" @operator)

(range_expression
  ":" @operator)

(arrow_function_expression
  "->" @operator)

[
  "."
  "..."
  "::"
] @punctuation.special

[
  ","
  ";"
] @punctuation.delimiter

[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
] @punctuation.bracket

; Keyword operators
((operator) @keyword.control
  (#any-of? @keyword.control "in" "isa"))

(where_expression
  "where" @keyword.control)

; Built-in constants
((identifier) @constant.builtin
  (#any-of? @constant.builtin "nothing" "missing"))

((identifier) @variable.special
  (#any-of? @variable.special "begin" "end")
  (#has-ancestor? @variable.special index_expression))

; Literals
(boolean_literal) @boolean

(integer_literal) @number

(float_literal) @number

((identifier) @number
  (#any-of? @number "NaN" "NaN16" "NaN32" "Inf" "Inf16" "Inf32"))

(character_literal) @string.special

(escape_sequence) @string.escape

(string_literal) @string

(prefixed_string_literal
  prefix: (identifier) @function.special) @string

(command_literal) @string.special

(prefixed_command_literal
  prefix: (identifier) @function.special) @string.special

((string_literal) @comment.doc
  .
  [
    (abstract_definition)
    (assignment)
    (const_statement)
    (function_definition)
    (macro_definition)
    (module_definition)
    (struct_definition)
  ])

[
  (line_comment)
  (block_comment)
] @comment
