use std::collections::{BTreeMap, BTreeSet};

use lpp_hir::{
    ArenaId, BinaryOperator, BindingTarget, BodyId, ExprId, ExpressionKind, FieldId, HirItem,
    HirItemId, HirItemKind, IdRange, Literal, LocalId, LocalKind, MatchArmId, ModuleId,
    NameBinding, OriginId, ScopeId, StatementKind, StmtId, Symbol, TypeParamId, UnaryOperator,
    VariantId,
};

use crate::{
    AggregateConstructor, AggregateExpressionFact, AugmentedAssignmentFact, BuiltinId,
    EnumMatchArmFact, EnumMatchFact, EnumTryFact, InferenceLevel, PlaceExpressionFact,
    PlaceStatementFact, PrimitiveType, TypeError, TypeId, TypeKind, TypeSubstitution,
};

use super::{ShadowTypeError, TypeChecker};

mod builtins;
mod capabilities;
mod inference;
mod intrinsics;
mod matches;
mod places;
mod safety;

fn blocking_builtin_label(name: &str) -> Option<&'static str> {
    Some(match name {
        "read_file" => "read_file",
        "write_file" => "write_file",
        "append_file" => "append_file",
        "delete_file" => "delete_file",
        "file_exists" => "file_exists",
        "read_line" => "read_line",
        "input" => "input",
        "file_size" => "file_size",
        "file_copy" => "file_copy",
        "file_move" => "file_move",
        "dir_create" => "dir_create",
        "dir_remove" => "dir_remove",
        "path_exists" => "path_exists",
        "command_output" => "command_output",
        "net_dial" => "net_dial",
        "net_dial_udp" => "net_dial_udp",
        "net_listen" => "net_listen",
        "net_listen_udp" => "net_listen_udp",
        "net_accept" => "net_accept",
        "net_accept_timeout" => "net_accept_timeout",
        "net_send" => "net_send",
        "net_send_all" => "net_send_all",
        "net_recv" => "net_recv",
        "net_recv_udp" => "net_recv_udp",
        "net_close" => "net_close",
        "net_set_deadline" => "net_set_deadline",
        "net_set_timeout" => "net_set_timeout",
        "net_set_nonblocking" => "net_set_nonblocking",
        "net_poll" => "net_poll",
        "net_set_keepalive" => "net_set_keepalive",
        "net_resolve" => "net_resolve",
        "net_connect" => "net_connect",
        "http_get" => "http_get",
        "http_post" => "http_post",
        "thread_join" => "thread_join",
        "sleep" => "sleep",
        "time_sleep" => "time_sleep",
        _ => return None,
    })
}

const fn is_comparison(operator: BinaryOperator) -> bool {
    matches!(
        operator,
        BinaryOperator::Equal
            | BinaryOperator::NotEqual
            | BinaryOperator::Less
            | BinaryOperator::Greater
            | BinaryOperator::LessEqual
            | BinaryOperator::GreaterEqual
            | BinaryOperator::LogicalAnd
            | BinaryOperator::LogicalOr
    )
}
