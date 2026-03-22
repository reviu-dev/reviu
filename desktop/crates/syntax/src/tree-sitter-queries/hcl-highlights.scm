(comment) @comment

(bool_lit) @boolean
(null_lit) @constant.builtin
(numeric_lit) @number

[
  (string_lit)
  (quoted_template)
  (heredoc_template)
  (template_literal)
  (quoted_template_start)
  (quoted_template_end)
  (heredoc_start)
  (heredoc_identifier)
] @string

(function_call
  (identifier) @function)

(attribute
  (identifier) @property)

(get_attr
  (identifier) @property)

(variable_expr
  (identifier) @variable)

(for_intro
  (identifier) @variable)

(template_for_start
  (identifier) @variable)

(block
  (identifier) @keyword)

[
  "for"
  "in"
  "if"
  "else"
  "endif"
  "endfor"
] @keyword

[
  "true"
  "false"
] @boolean

[
  "!"
  "!="
  "%"
  "&&"
  "*"
  "+"
  "-"
  "/"
  "<"
  "<="
  "="
  "=="
  "=>"
  ">"
  ">="
  "?"
  "||"
] @operator

[
  ","
  "."
  ":"
] @punctuation.delimiter

[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
  (block_start)
  (block_end)
  (object_start)
  (object_end)
  (tuple_start)
  (tuple_end)
] @punctuation.bracket

[
  (template_directive_start)
  (template_directive_end)
  (template_interpolation_start)
  (template_interpolation_end)
  (strip_marker)
  (ellipsis)
] @punctuation.special
