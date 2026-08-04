use la_arena::Idx;
use num_bigint::BigInt;
use smol_str::SmolStr;
use tree_sitter::Node;

use crate::ast::kinds::{FieldKind, NodeKind};
use crate::ast::{AstNode, NodeRange};
use crate::hir::body_map::Location;
use crate::hir::types::{LiteralType, Type, TypeKey};

pub type Name = SmolStr;
pub type ExprId = Idx<Expr>;//This forces builder to use Arenas for exprs, Ideally the builder should be able to store & id exprs however they want

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    And,
    Or,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

impl BinaryOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Mod => "%",
            Self::Pow => "**",
            Self::BitAnd => "&",
            Self::BitOr => "|",
            Self::BitXor => "^",
            Self::Shl => "<<",
            Self::Shr => ">>",
            Self::And => "&&",
            Self::Or => "||",
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::Lt => "<",
            Self::Gt => ">",
            Self::Le => "<=",
            Self::Ge => ">=",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "+" => Self::Add,
            "-" => Self::Sub,
            "*" => Self::Mul,
            "/" => Self::Div,
            "%" => Self::Mod,
            "**" => Self::Pow,
            "&" => Self::BitAnd,
            "|" => Self::BitOr,
            "^" => Self::BitXor,
            "<<" => Self::Shl,
            ">>" => Self::Shr,
            "&&" => Self::And,
            "||" => Self::Or,
            "==" => Self::Eq,
            "!=" => Self::Ne,
            "<" => Self::Lt,
            ">" => Self::Gt,
            "<=" => Self::Le,
            ">=" => Self::Ge,
            _ => return None,
        })
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum Expr {
    Ident(Name),
    Path(Box<str>),//put path in literal??
    Literal(Literal),
    Binary {
        op: BinaryOp,
        left: ExprId,
        right: ExprId,
    },
    Member {
        obj: ExprId,
        prop: Name,
    },
    Call {
        callee: ExprId,
        args: Box<[ExprId]>
    },
    /// NOTE: this also covers mapping access exprs ie. balance[msg.sender]
    ArrayAccess {
        base: ExprId,
        index: Option<ExprId>,
    },
    MetaType(ExprId)
}

#[derive(Clone, PartialEq, Eq)]
pub enum Literal {
    Boolean(bool),
    String(SmolStr),
    Number(SmolStr),//store numbers as u256
    HexString(SmolStr),
}

impl Literal {
    fn parse_number(text: &str) -> LiteralType {
        let text = text.trim().replace('_', "");
        if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
            if let Some(value) = BigInt::parse_bytes(hex.as_bytes(), 16) {
                return LiteralType::Integer(value);
            }
        }

        let (mantissa, exponent) = text
            .split_once(['e', 'E'])
            .map(|(mantissa, exponent)| (mantissa, exponent.parse::<i32>().unwrap_or(0)))
            .unwrap_or((&text, 0));
        let (negative, mantissa) = mantissa.strip_prefix('-')
            .map(|value| (true, value))
            .or_else(|| mantissa.strip_prefix('+').map(|value| (false, value)))
            .unwrap_or((false, mantissa));
        let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        let digits = format!("{whole}{fraction}");
        let mut numerator = BigInt::parse_bytes(digits.as_bytes(), 10).unwrap_or_default();
        if negative {
            numerator = -numerator;
        }

        let scale = fraction.len() as i32 - exponent;
        if scale <= 0 {
            for _ in 0..(-scale) {
                numerator *= 10;
            }
            LiteralType::Integer(numerator)
        } else {
            let mut denominator = BigInt::from(1u8);
            for _ in 0..scale {
                denominator *= 10;
            }
            LiteralType::Rational { numerator, denominator }
        }
    }

    fn literal_type(&self) -> LiteralType {
        match self {
            Literal::Boolean(value) => LiteralType::Boolean(*value),
            Literal::Number(value) => Self::parse_number(value),
            Literal::String(value) => LiteralType::String(value.clone()),
            Literal::HexString(value) => LiteralType::HexString(value.clone()),
        }
    }

    pub fn loc(&self) -> Location {
        match self {
            Literal::String(_) | Literal::HexString(_) => Location::Memory,
            _ => Location::Stack,
        }
    }

    pub fn type_key(&self) -> TypeKey {
        let literal = self.literal_type();
        TypeKey(Type::Literal(literal), self.loc())
    }
}

pub trait  ExprBuilder {
    fn root(&self) -> &AstNode;
    fn alloc_expr(&mut self, expr: Expr, node: Node) -> ExprId;
    fn alloc_member_expr(&mut self, expr: Expr, range: NodeRange, node: Node) -> ExprId;
    fn alloc_call_expr(&mut self, call: Expr, callee_node: Node, node: Node) -> ExprId;


    fn lower_expr(&mut self, node: Node) -> Option<ExprId> {
        match node.kind_id().into() {
            NodeKind::NUMBER_LITERAL => {
                let text = self.root().text_by_range(node.byte_range());
                Some(self.alloc_expr(Expr::Literal(Literal::Number(text.into())), node))
            }
            NodeKind::HEX_STRING_LITERAL => {
                let text = self.root().text_by_range(node.byte_range());
                Some(self.alloc_expr(Expr::Literal(Literal::HexString(text.into())), node))
            }
            NodeKind::BOOLEAN_LITERAL => {
                let text = self.root().text_by_range(node.byte_range()).trim();
                Some(self.alloc_expr(Expr::Literal(Literal::Boolean(text == "true")), node))
            }
            NodeKind::STRING_LITERAL => {
                let text = self.root().text_by_range(node.byte_range());
                Some(self.alloc_expr(Expr::Literal(Literal::String(text.into())), node))
            },
            //types(primitive/user defined) contained in expressions are lowered as identifiers. 
            //This is mainly to support type casts & udvt's e.g address(this)
            NodeKind::IDENTIFIER | NodeKind::PRIMITIVE_TYPE => {
                let ident = self.root().text_by_range(node.byte_range());
                Some(self.alloc_expr(Expr::Ident(ident.into()), node))

            }
            NodeKind::META_TYPE_EXPRESSION => {
                let type_node = node.child(0)?;//type is an unnamed node
                let ty = self.lower_expr(node.named_child(0)?)?;

                let expr = Expr::MetaType(ty);
                // still alloc as call because of shape
                Some(self.alloc_call_expr(expr, type_node, node))
            }
            NodeKind::MEMBER_EXPRESSION => {
                let obj = node.child_by_field_id(FieldKind::OBJECT.into()).and_then(|obj| self.lower_expr(obj))?;
                
                let (name, range) = node.child_by_field_id(FieldKind::PROPERTY.into()).map(|prop| (self.root().text_by_range(prop.byte_range()), NodeRange::from(&prop)))?;

                Some(self.alloc_member_expr(Expr::Member { obj, prop: name.into() }, range, node))
                
            }
            NodeKind::CALL_EXPRESSION => {
                let callee_node = node.child_by_field_id(FieldKind::FUNCTION.into())?;
                let callee = self.lower_expr(callee_node)?;

                let args = node.named_children(&mut node.walk()).filter_map(|n| {
                    if n.kind_id() == NodeKind::CALL_ARGUMENT {
                        self.lower_expr(n)
                    } else {
                        None
                    }
                }).collect::<Box<_>>();

                let call_expr = Expr::Call {
                    callee,
                    args,
                };
                Some(self.alloc_call_expr(call_expr, callee_node, node))
            }
            NodeKind::BINARY_EXPRESSION => {
                let left = node.named_child(0).and_then(|n| self.lower_expr(n));
                let right = node.named_child(1).and_then(|n| self.lower_expr(n));
                let op = node.child_by_field_id(FieldKind::OPERATOR.into())
                    .map(|op| self.root().text_by_range(op.byte_range()).trim())
                    .and_then(|s| BinaryOp::parse(s))?;
                Some(self.alloc_expr(Expr::Binary { op, left: left?, right: right? }, node))//late ? so both sides can fully lower.
            }
            NodeKind::ARRAY_ACCESS => {
                // we allow index to be optional but not obj
                let base = node.child_by_field_id(FieldKind::BASE.into()).and_then(|obj| self.lower_expr(obj))?;
                
                let index = node.child_by_field_id(FieldKind::INDEX.into()).and_then(|index| self.lower_expr(index));
                
                let array_expr = Expr::ArrayAccess {
                    base,
                    index,
                };
                Some(self.alloc_expr(array_expr, node))
            }
            NodeKind::EMIT_STATEMENT => {
                //Emit statements are structured like expressions but not wrapped in an expression 🤷‍♂️
                let callee_node = node.child_by_field_id(FieldKind::NAME.into())?;
                let callee = self.lower_expr(callee_node)?;

                let args = node.named_children(&mut node.walk()).filter_map(|n| {
                    if n.kind_id() == NodeKind::CALL_ARGUMENT {
                        self.lower_expr(n)
                    } else {
                        None
                    }
                }).collect::<Box<_>>();

                let call_expr = Expr::Call {
                    callee,
                    args,
                };
                Some(self.alloc_call_expr(call_expr, callee_node, node))
                
            }
            NodeKind::REVERT_STATEMENT => {
                let callee_node = node.named_child(0)?;
                let callee = self.lower_expr(callee_node)?;

                let args_node = node.named_children(&mut node.walk()).find(|n| n.kind_id() == NodeKind::REVERT_ARGUMENTS)?;

                let args = args_node.named_children(&mut args_node.walk()).filter_map(|n| {
                    if n.kind_id() == NodeKind::CALL_ARGUMENT {
                        self.lower_expr(n)
                    } else {
                        None
                    }
                }).collect::<Box<_>>();
                
                let call_expr = Expr::Call {
                    callee,
                    args,
                };
                Some(self.alloc_call_expr(call_expr, callee_node, node))
            }
            NodeKind::MODIFIER_INVOCATION | NodeKind::TYPECAST_EXPRESSION =>  {
                let name_node = node.named_child(0)?;
                let callee = self.lower_expr(name_node)?;

                let args = node.named_children(&mut node.walk()).filter_map(|n| {
                    if n.kind_id() == NodeKind::CALL_ARGUMENT {
                        self.lower_expr(n)
                    } else {
                        None
                    }
                }).collect::<Box<_>>();
                
                let call_expr = Expr::Call {
                    callee,
                    args,
                };
                Some(self.alloc_call_expr(call_expr, name_node, node))
            }
            _ => {
                //intermediate exprs e.g.
                /*
                expression: "x"
                    {type_of}_expression: "x"
                 */
                /*
                call_argument: "x"
                    expression: "x"
                        identifier: "x"
                 */

                // and exprs we don't lower directly e.g.tuple_expression
                // if single child we bubble the id up
                match node.named_child_count() {
                    1 => self.lower_expr(node.named_child(0).unwrap()), 
                    _ => {
                        for child in node.named_children(&mut node.walk()) {
                            self.lower_expr(child);
                        }
                        None
                    }
                }
            }
        }
    }
}