use gpui::Hsla;

#[derive(Debug, Clone)]
pub struct Theme {
  pub is_dark: bool,
}

impl Theme {
  pub fn new(is_dark_mode: bool) -> Self {
    Self {
      is_dark: is_dark_mode,
    }
  }

  pub fn dark() -> Self {
    Self::new(true)
  }

  pub fn light() -> Self {
    Self::new(false)
  }

  pub fn toggle(&mut self) {
    self.is_dark = !self.is_dark;
  }

  pub fn syntax(&self) -> SyntaxTheme {
    if self.is_dark {
      SyntaxTheme::default_dark()
    } else {
      SyntaxTheme::default_light()
    }
  }

  pub fn cursor(&self) -> Hsla {
    Hsla {
      h: 210.0 / 360.0,
      s: 1.0,
      l: 0.5,
      a: 0.7,
    }
  }

  pub fn gutter_background(&self) -> Hsla {
    if self.is_dark {
      Hsla {
        h: 0.0,
        s: 0.0,
        l: 0.12,
        a: 1.0,
      } // #1e1e1e
    } else {
      Hsla {
        h: 0.0,
        s: 0.0,
        l: 0.96,
        a: 1.0,
      } // #f5f5f5
    }
  }

  pub fn line_number(&self) -> Hsla {
    if self.is_dark {
      Hsla {
        h: 0.0,
        s: 0.0,
        l: 0.53,
        a: 1.0,
      } // #888888
    } else {
      Hsla {
        h: 0.0,
        s: 0.0,
        l: 0.40,
        a: 1.0,
      } // #666666
    }
  }

  pub fn selection(&self) -> Hsla {
    if self.is_dark {
      Hsla {
        h: 210.0 / 360.0,
        s: 1.0,
        l: 0.55,
        a: 0.3,
      }
    } else {
      Hsla {
        h: 210.0 / 360.0,
        s: 1.0,
        l: 0.85,
        a: 0.4,
      }
    }
  }

  pub fn diff_added_background(&self) -> Hsla {
    if self.is_dark {
      Hsla {
        h: 120.0 / 360.0,
        s: 0.35,
        l: 0.22,
        a: 0.6,
      }
    } else {
      Hsla {
        h: 120.0 / 360.0,
        s: 0.35,
        l: 0.9,
        a: 0.7,
      }
    }
  }

  pub fn diff_removed_background(&self) -> Hsla {
    if self.is_dark {
      Hsla {
        h: 0.0,
        s: 0.45,
        l: 0.22,
        a: 0.6,
      }
    } else {
      Hsla {
        h: 0.0,
        s: 0.45,
        l: 0.9,
        a: 0.7,
      }
    }
  }

  pub fn diff_added_word_background(&self) -> Hsla {
    if self.is_dark {
      Hsla {
        h: 120.0 / 360.0,
        s: 0.55,
        l: 0.3,
        a: 0.85,
      }
    } else {
      Hsla {
        h: 120.0 / 360.0,
        s: 0.6,
        l: 0.8,
        a: 0.9,
      }
    }
  }

  pub fn diff_removed_word_background(&self) -> Hsla {
    if self.is_dark {
      Hsla {
        h: 0.0,
        s: 0.6,
        l: 0.3,
        a: 0.85,
      }
    } else {
      Hsla {
        h: 0.0,
        s: 0.6,
        l: 0.8,
        a: 0.9,
      }
    }
  }

  pub fn diff_gutter_added(&self) -> Hsla {
    Hsla {
      h: 120.0 / 360.0,
      s: 0.6,
      l: 0.45,
      a: 1.0,
    }
  }

  pub fn diff_gutter_modified(&self) -> Hsla {
    Hsla {
      h: 30.0 / 360.0,
      s: 0.75,
      l: 0.5,
      a: 1.0,
    }
  }
}

impl Default for Theme {
  fn default() -> Self {
    Self::dark()
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenType {
  Keyword,
  KeywordControl,
  Function,
  FunctionMethod,
  FunctionSpecial,
  Type,
  TypeBuiltin,
  TypeInterface,
  TypeClass,
  String,
  StringEscape,
  StringRegex,
  Number,
  Boolean,
  Comment,
  CommentDoc,
  Operator,
  Variable,
  VariableSpecial,
  VariableParameter,
  Property,
  Constant,
  ConstantBuiltin,
  Punctuation,
  PunctuationBracket,
  PunctuationDelimiter,
  PunctuationSpecial,
  Attribute,
  Lifetime,
  Embedded,
}

/// Syntax highlighting theme
#[derive(Debug, Clone)]
pub struct SyntaxTheme {
  pub keyword: Hsla,
  pub keyword_control: Hsla,
  pub function: Hsla,
  pub function_method: Hsla,
  pub function_special: Hsla,
  pub type_name: Hsla,
  pub type_builtin: Hsla,
  pub type_interface: Hsla,
  pub type_class: Hsla,
  pub string: Hsla,
  pub string_escape: Hsla,
  pub string_regex: Hsla,
  pub number: Hsla,
  pub boolean: Hsla,
  pub comment: Hsla,
  pub comment_doc: Hsla,
  pub operator: Hsla,
  pub variable: Hsla,
  pub variable_special: Hsla,
  pub variable_parameter: Hsla,
  pub property: Hsla,
  pub constant: Hsla,
  pub constant_builtin: Hsla,
  pub punctuation: Hsla,
  pub punctuation_bracket: Hsla,
  pub punctuation_delimiter: Hsla,
  pub punctuation_special: Hsla,
  pub attribute: Hsla,
  pub lifetime: Hsla,
  pub embedded: Hsla,
}

impl SyntaxTheme {
  pub fn color_for_token(&self, token_type: TokenType) -> Hsla {
    match token_type {
      TokenType::Keyword => self.keyword,
      TokenType::KeywordControl => self.keyword_control,
      TokenType::Function => self.function,
      TokenType::FunctionMethod => self.function_method,
      TokenType::FunctionSpecial => self.function_special,
      TokenType::Type => self.type_name,
      TokenType::TypeBuiltin => self.type_builtin,
      TokenType::TypeInterface => self.type_interface,
      TokenType::TypeClass => self.type_class,
      TokenType::String => self.string,
      TokenType::StringEscape => self.string_escape,
      TokenType::StringRegex => self.string_regex,
      TokenType::Number => self.number,
      TokenType::Boolean => self.boolean,
      TokenType::Comment => self.comment,
      TokenType::CommentDoc => self.comment_doc,
      TokenType::Operator => self.operator,
      TokenType::Variable => self.variable,
      TokenType::VariableSpecial => self.variable_special,
      TokenType::VariableParameter => self.variable_parameter,
      TokenType::Property => self.property,
      TokenType::Constant => self.constant,
      TokenType::ConstantBuiltin => self.constant_builtin,
      TokenType::Punctuation => self.punctuation,
      TokenType::PunctuationBracket => self.punctuation_bracket,
      TokenType::PunctuationDelimiter => self.punctuation_delimiter,
      TokenType::PunctuationSpecial => self.punctuation_special,
      TokenType::Attribute => self.attribute,
      TokenType::Lifetime => self.lifetime,
      TokenType::Embedded => self.embedded,
    }
  }
}

impl SyntaxTheme {
  /// Default dark theme inspired by VS Code Dark+
  pub fn default_dark() -> Self {
    Self {
      // Keywords - blue
      keyword: Hsla {
        h: 210.0 / 360.0,
        s: 0.59,
        l: 0.63,
        a: 1.0,
      }, // #569cd6
      keyword_control: Hsla {
        h: 291.0 / 360.0,
        s: 0.47,
        l: 0.63,
        a: 1.0,
      }, // #c586c0

      // Functions - yellow
      function: Hsla {
        h: 50.0 / 360.0,
        s: 0.61,
        l: 0.71,
        a: 1.0,
      }, // #dcdcaa
      function_method: Hsla {
        h: 50.0 / 360.0,
        s: 0.61,
        l: 0.71,
        a: 1.0,
      }, // #dcdcaa
      function_special: Hsla {
        h: 210.0 / 360.0,
        s: 0.59,
        l: 0.63,
        a: 1.0,
      }, // #569cd6 (macros in blue)

      // Types - cyan/turquoise
      type_name: Hsla {
        h: 167.0 / 360.0,
        s: 0.49,
        l: 0.59,
        a: 1.0,
      }, // #4ec9b0
      type_builtin: Hsla {
        h: 167.0 / 360.0,
        s: 0.49,
        l: 0.59,
        a: 1.0,
      }, // #4ec9b0
      type_interface: Hsla {
        h: 167.0 / 360.0,
        s: 0.49,
        l: 0.59,
        a: 1.0,
      }, // #4ec9b0
      type_class: Hsla {
        h: 167.0 / 360.0,
        s: 0.49,
        l: 0.59,
        a: 1.0,
      }, // #4ec9b0

      // Strings - orange
      string: Hsla {
        h: 25.0 / 360.0,
        s: 0.51,
        l: 0.63,
        a: 1.0,
      }, // #ce9178
      string_escape: Hsla {
        h: 50.0 / 360.0,
        s: 0.61,
        l: 0.71,
        a: 1.0,
      }, // #d7ba7d
      string_regex: Hsla {
        h: 341.0 / 360.0,
        s: 0.79,
        l: 0.68,
        a: 1.0,
      }, // #d16969

      // Numbers - light green
      number: Hsla {
        h: 99.0 / 360.0,
        s: 0.28,
        l: 0.71,
        a: 1.0,
      }, // #b5cea8
      boolean: Hsla {
        h: 210.0 / 360.0,
        s: 0.59,
        l: 0.63,
        a: 1.0,
      }, // #569cd6

      // Comments - green/gray
      comment: Hsla {
        h: 113.0 / 360.0,
        s: 0.23,
        l: 0.49,
        a: 1.0,
      }, // #6a9955
      comment_doc: Hsla {
        h: 113.0 / 360.0,
        s: 0.23,
        l: 0.55,
        a: 1.0,
      }, // #6a9955 (slightly brighter)

      // Operators - white
      operator: Hsla {
        h: 0.0,
        s: 0.0,
        l: 0.86,
        a: 1.0,
      }, // #d4d4d4

      // Variables
      variable: Hsla {
        h: 215.0 / 360.0,
        s: 0.76,
        l: 0.78,
        a: 1.0,
      }, // #9cdcfe
      variable_special: Hsla {
        h: 210.0 / 360.0,
        s: 0.59,
        l: 0.63,
        a: 1.0,
      }, // #569cd6 (this, self)
      variable_parameter: Hsla {
        h: 215.0 / 360.0,
        s: 0.76,
        l: 0.78,
        a: 1.0,
      }, // #9cdcfe

      // Properties
      property: Hsla {
        h: 215.0 / 360.0,
        s: 0.76,
        l: 0.78,
        a: 1.0,
      }, // #9cdcfe

      // Constants
      constant: Hsla {
        h: 215.0 / 360.0,
        s: 0.76,
        l: 0.78,
        a: 1.0,
      }, // #9cdcfe
      constant_builtin: Hsla {
        h: 210.0 / 360.0,
        s: 0.59,
        l: 0.63,
        a: 1.0,
      }, // #569cd6 (true, false, null, etc.)

      // Punctuation - light gray
      punctuation: Hsla {
        h: 0.0,
        s: 0.0,
        l: 0.83,
        a: 1.0,
      }, // #d4d4d4
      punctuation_bracket: Hsla {
        h: 50.0 / 360.0,
        s: 0.61,
        l: 0.71,
        a: 1.0,
      }, // #ffd700 (brackets in gold)
      punctuation_delimiter: Hsla {
        h: 0.0,
        s: 0.0,
        l: 0.83,
        a: 1.0,
      }, // #d4d4d4
      punctuation_special: Hsla {
        h: 210.0 / 360.0,
        s: 0.59,
        l: 0.63,
        a: 1.0,
      }, // #569cd6

      // Attributes/Decorators - yellow
      attribute: Hsla {
        h: 50.0 / 360.0,
        s: 0.61,
        l: 0.71,
        a: 1.0,
      }, // #dcdcaa

      // Lifetime (Rust) - cyan
      lifetime: Hsla {
        h: 167.0 / 360.0,
        s: 0.49,
        l: 0.59,
        a: 1.0,
      }, // #4ec9b0

      // Embedded (template strings)
      embedded: Hsla {
        h: 215.0 / 360.0,
        s: 0.76,
        l: 0.78,
        a: 1.0,
      }, // #9cdcfe
    }
  }

  /// Default light theme
  pub fn default_light() -> Self {
    Self {
      // Keywords - darker blue
      keyword: Hsla {
        h: 210.0 / 360.0,
        s: 0.79,
        l: 0.35,
        a: 1.0,
      }, // #0000FF
      keyword_control: Hsla {
        h: 291.0 / 360.0,
        s: 0.67,
        l: 0.40,
        a: 1.0,
      }, // #AF00DB

      // Functions - darker brown/gold
      function: Hsla {
        h: 35.0 / 360.0,
        s: 0.75,
        l: 0.35,
        a: 1.0,
      }, // #795E26
      function_method: Hsla {
        h: 35.0 / 360.0,
        s: 0.75,
        l: 0.35,
        a: 1.0,
      }, // #795E26
      function_special: Hsla {
        h: 210.0 / 360.0,
        s: 0.79,
        l: 0.35,
        a: 1.0,
      }, // #0000FF (macros in blue)

      // Types - teal/cyan
      type_name: Hsla {
        h: 180.0 / 360.0,
        s: 0.69,
        l: 0.31,
        a: 1.0,
      }, // #267F99
      type_builtin: Hsla {
        h: 180.0 / 360.0,
        s: 0.69,
        l: 0.31,
        a: 1.0,
      }, // #267F99
      type_interface: Hsla {
        h: 180.0 / 360.0,
        s: 0.69,
        l: 0.31,
        a: 1.0,
      }, // #267F99
      type_class: Hsla {
        h: 180.0 / 360.0,
        s: 0.69,
        l: 0.31,
        a: 1.0,
      }, // #267F99

      // Strings - dark orange/red
      string: Hsla {
        h: 5.0 / 360.0,
        s: 0.73,
        l: 0.38,
        a: 1.0,
      }, // #A31515
      string_escape: Hsla {
        h: 30.0 / 360.0,
        s: 0.65,
        l: 0.45,
        a: 1.0,
      }, // #EE9900
      string_regex: Hsla {
        h: 341.0 / 360.0,
        s: 0.69,
        l: 0.48,
        a: 1.0,
      }, // #D16969

      // Numbers - dark green
      number: Hsla {
        h: 120.0 / 360.0,
        s: 0.53,
        l: 0.31,
        a: 1.0,
      }, // #098658
      boolean: Hsla {
        h: 210.0 / 360.0,
        s: 0.79,
        l: 0.35,
        a: 1.0,
      }, // #0000FF

      // Comments - green
      comment: Hsla {
        h: 120.0 / 360.0,
        s: 0.43,
        l: 0.35,
        a: 1.0,
      }, // #008000
      comment_doc: Hsla {
        h: 120.0 / 360.0,
        s: 0.43,
        l: 0.40,
        a: 1.0,
      }, // #008000 (slightly brighter)

      // Operators - dark gray
      operator: Hsla {
        h: 0.0,
        s: 0.0,
        l: 0.20,
        a: 1.0,
      }, // #333333

      // Variables - dark blue/black
      variable: Hsla {
        h: 210.0 / 360.0,
        s: 0.76,
        l: 0.26,
        a: 1.0,
      }, // #001080
      variable_special: Hsla {
        h: 210.0 / 360.0,
        s: 0.79,
        l: 0.35,
        a: 1.0,
      }, // #0000FF (this, self)
      variable_parameter: Hsla {
        h: 210.0 / 360.0,
        s: 0.76,
        l: 0.26,
        a: 1.0,
      }, // #001080

      // Properties
      property: Hsla {
        h: 210.0 / 360.0,
        s: 0.76,
        l: 0.26,
        a: 1.0,
      }, // #001080

      // Constants
      constant: Hsla {
        h: 210.0 / 360.0,
        s: 0.76,
        l: 0.26,
        a: 1.0,
      }, // #001080
      constant_builtin: Hsla {
        h: 210.0 / 360.0,
        s: 0.79,
        l: 0.35,
        a: 1.0,
      }, // #0000FF (true, false, null, etc.)

      // Punctuation - dark gray
      punctuation: Hsla {
        h: 0.0,
        s: 0.0,
        l: 0.20,
        a: 1.0,
      }, // #333333
      punctuation_bracket: Hsla {
        h: 35.0 / 360.0,
        s: 0.75,
        l: 0.35,
        a: 1.0,
      }, // #795E26 (brackets in brown/gold)
      punctuation_delimiter: Hsla {
        h: 0.0,
        s: 0.0,
        l: 0.20,
        a: 1.0,
      }, // #333333
      punctuation_special: Hsla {
        h: 210.0 / 360.0,
        s: 0.79,
        l: 0.35,
        a: 1.0,
      }, // #0000FF

      // Attributes/Decorators - brown/gold
      attribute: Hsla {
        h: 35.0 / 360.0,
        s: 0.75,
        l: 0.35,
        a: 1.0,
      }, // #795E26

      // Lifetime (Rust) - teal
      lifetime: Hsla {
        h: 180.0 / 360.0,
        s: 0.69,
        l: 0.31,
        a: 1.0,
      }, // #267F99

      // Embedded (template strings)
      embedded: Hsla {
        h: 210.0 / 360.0,
        s: 0.76,
        l: 0.26,
        a: 1.0,
      }, // #001080
    }
  }
}

impl Default for SyntaxTheme {
  fn default() -> Self {
    Self::default_dark()
  }
}
