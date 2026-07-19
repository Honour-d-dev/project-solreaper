use la_arena::Idx;
use tree_sitter::Node;
use smol_str::SmolStr;

use crate::ast::kinds::{FieldKind, NodeKind};
use crate::ast::{AstNode, NodeRange};

pub type Name = SmolStr;
pub type ExprId = Idx<Expr>;//This forces builder to use Arenas for exprs, Ideally the builder should be able to store & id exprs however they want

#[derive(PartialEq, Eq)]
pub enum Expr {
    Ident(Name),
    Literal(Literal),
    Member {
        obj: ExprId,
        prop: Name,
    },
    Call {
        callee: ExprId,
        args: Box<[ExprId]>
    },
    /// NOTE: this also covers mapping access exprs ie. balance[msg.sender]
    Array {
        base: ExprId,
        index: Option<ExprId>,
    }
}

#[derive(PartialEq, Eq)]
pub enum Literal {
    Boolean(NodeRange),
    String(NodeRange),
    Number(NodeRange),
    HexString(NodeRange),
}

pub trait  ExprBuilder {
    fn root(&self) -> &AstNode;
    fn alloc_expr(&mut self, expr: Expr, node: Node) -> ExprId;
    fn alloc_member_expr(&mut self, expr: Expr, range: NodeRange, node: Node) -> ExprId;

    fn walk_expr(&mut self, node: Node) -> Option<ExprId> {
        // unwraping/walking nested expressions
        // expressions only have 1 child which is the type of the expression
        match node.named_child_count() {
            1 => self.lower_expr(node.named_child(0).unwrap()), 
            _ => None
        }
        
    }


    fn lower_expr(&mut self, node: Node) -> Option<ExprId> {
        match node.kind_id().into() {
            NodeKind::EXPRESSION => {
                /*
                expression: "x"
                    {type_of}_expression: "x"
                 */
                self.walk_expr(node)
            }
            NodeKind::TUPLE_EXPRESSION => {
                for expr in node.named_children(&mut  node.walk()) {
                    self.lower_expr(expr);
                }
                //Nono for now, yet to decide if we want to assign id to this
                //FIXME: i think the expr walking algo can be cleaner insead of walk+lower
                None
            }
            NodeKind::NUMBER_LITERAL => Some(self.alloc_expr(Expr::Literal(Literal::Number(NodeRange::from(&node))), node)),
            NodeKind::HEX_STRING_LITERAL => Some(self.alloc_expr(Expr::Literal(Literal::HexString(NodeRange::from(&node))), node)),
            NodeKind::BOOLEAN_LITERAL => Some(self.alloc_expr(Expr::Literal(Literal::Boolean(NodeRange::from(&node))), node)),
            NodeKind::STRING_LITERAL => Some(self.alloc_expr(Expr::Literal(Literal::String(NodeRange::from(&node))), node)),
            NodeKind::IDENTIFIER => {
                let ident = self.root().text_by_range(node.byte_range());
                Some(self.alloc_expr(Expr::Ident(ident.into()), node))

            }
            NodeKind::MEMBER_EXPRESSION => {
                let obj = node.child_by_field_id(FieldKind::OBJECT.into()).and_then(|obj| self.lower_expr(obj))?;
                
                let (name, range) = node.child_by_field_id(FieldKind::PROPERTY.into()).map(|prop| (self.root().text_by_range(prop.byte_range()), NodeRange::from(&prop)))?;

                Some(self.alloc_member_expr(Expr::Member { obj, prop: name.into() }, range, node))
                
                //Is the prev exprId before the prop always the obj? is it an invariant??
            }
            NodeKind::CALL_EXPRESSION => {
                let callee = node.child_by_field_id(FieldKind::FUNCTION.into()).and_then(|n| self.lower_expr(n))?;

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
                Some(self.alloc_expr(call_expr, node))
            }
            NodeKind::CALL_ARGUMENT => {
                /*
                call_argument: "x"
                    expression: "x"
                        identifier: "x"
                 */
                self.walk_expr(node)
            }
            NodeKind::ARRAY_ACCESS => {
                // we allow index to be optional but not obj
                let base = node.child_by_field_id(FieldKind::BASE.into()).and_then(|obj| self.lower_expr(obj))?;
                
                let index = node.child_by_field_id(FieldKind::INDEX.into()).and_then(|index| self.lower_expr(index));
                
                let array_expr = Expr::Array {
                    base,
                    index,
                };
                Some(self.alloc_expr(array_expr, node))
            }
            _ => {
                //must be some kind of intermediate/not supported expr
                //lower children exprs
                for child in node.named_children(&mut node.walk()) {
                    self.lower_expr(child);
                }
                None
            }
        }
    }
}