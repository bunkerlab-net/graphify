//! Parity port of `graphify-py/tests/test_dart.py`.
//!
//! Exercises the modernised Dart extractor: generics, inheritance, mixins,
//! interfaces, annotations, Riverpod/Bloc codegen, extensions, typedefs,
//! navigation, `part of` redirection, and generic type-lookup invocations.
//!
//! Where Python asserts `source_file is None` for a global reference node, the
//! Rust port carries an empty `source_file` string (the `Node` type is
//! non-optional), so those assertions check `.source_file.is_empty()`.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::too_many_lines)]

use graphify_extract::{Edge, Node, extract_dart, make_id1};

fn write_dart(dir: &std::path::Path, name: &str, code: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, code).expect("write dart fixture");
    p
}

fn node(nodes: &[Node], pred: impl Fn(&Node) -> bool) -> Option<&Node> {
    nodes.iter().find(|n| pred(n))
}

fn edge(edges: &[Edge], pred: impl Fn(&Edge) -> bool) -> Option<&Edge> {
    edges.iter().find(|e| pred(e))
}

#[test]
fn universal_generic_syntax_extraction() {
    let code = "
import 'package:flutter/material.dart';
import 'package:flutter_bloc/flutter_bloc.dart';
import 'package:injectable/injectable.dart';
export 'package:flutter_bloc/flutter_bloc.dart';

// 1. Class declarations with generics, inheritance, and implements
@injectable
@HiveType(typeId: 10)
class UserBloc extends Bloc<UserEvent, UserState> with MyMixin implements Disposable {
  UserBloc() : super(InitialState());
}


// 2. Enum declarations
@jsonSerializable
enum UserRole { admin, user }

// 3. Extensions
extension StringExtensions on String {
  bool get isEmail => contains('@');
}

// 4. Top-level variables
final authServiceProvider = Provider<AuthService>((ref) => AuthService());
final myData = 42;

// 5. Generic method invocations (automatically catches GetIt, Provider, BlocProvider, InheritedWidget!)
void checkDependencies(BuildContext context) {
  final custom = context.dependOnInheritedWidgetOfExactType<CustomService>();
  final auth = context.read<AuthService>();
  final bloc = BlocProvider.of<UserBloc>(context);
  final getItService = GetIt.I<DatabaseService>();
  final locatorService = locator<api.NetworkFactory>();

}
";
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = write_dart(tmp.path(), "test_app_bloc.dart", code);
    let result = extract_dart(&path);
    let nodes = &result.nodes;
    let edges = &result.edges;
    let str_path = path.to_string_lossy().into_owned();

    // A. File node
    let file_node = node(nodes, |n| {
        n.file_type == "code" && n.label == "test_app_bloc.dart"
    })
    .expect("file node");
    assert_eq!(file_node.source_file, str_path);

    // B. Class & enum
    let user_bloc = node(nodes, |n| n.label == "UserBloc").expect("UserBloc node");
    assert_eq!(user_bloc.source_file, str_path);
    assert!(node(nodes, |n| n.label == "UserRole").is_some());

    // C. Inherits + generics (global ids, empty source_file)
    let inherits = edge(edges, |e| {
        e.source == user_bloc.id && e.relation == "inherits"
    })
    .expect("inherits edge");
    assert_eq!(inherits.target, "bloc");
    let bloc_node = node(nodes, |n| n.id == "bloc").expect("bloc node");
    assert!(bloc_node.source_file.is_empty());

    assert!(
        edge(edges, |e| e.source == user_bloc.id
            && e.relation == "references"
            && e.target == "userevent")
        .is_some()
    );
    let event_node = node(nodes, |n| n.id == "userevent").expect("userevent node");
    assert!(event_node.source_file.is_empty());
    assert!(
        edge(edges, |e| e.source == user_bloc.id
            && e.relation == "references"
            && e.target == "userstate")
        .is_some()
    );

    // D. Class annotation (global id, empty source_file) + configures edge
    let injectable = node(nodes, |n| n.label == "@injectable").expect("@injectable node");
    assert_eq!(injectable.id, "annotation_injectable");
    assert!(injectable.source_file.is_empty());
    assert!(
        edge(edges, |e| e.source == user_bloc.id
            && e.target == injectable.id
            && e.relation == "configures")
        .is_some()
    );

    // Mixin
    assert!(
        edge(edges, |e| e.source == user_bloc.id
            && e.target == "mymixin"
            && e.relation == "implements")
        .is_some()
    );

    // E. Extension + extends-target string
    let ext_node = node(nodes, |n| n.label == "StringExtensions").expect("extension node");
    let extends = edge(edges, |e| {
        e.source == ext_node.id && e.relation == "extends"
    })
    .expect("extends edge");
    assert_eq!(extends.target, "string");

    // F. Variable declaration
    assert!(node(nodes, |n| n.label == "authServiceProvider").is_some());

    // G. Universal generic invocation mappings (file-sourced edges)
    assert!(
        edge(edges, |e| e.source == file_node.id
            && e.target == "customservice"
            && e.relation == "references")
        .is_some()
    );
    let custom = node(nodes, |n| n.id == "customservice").expect("customservice node");
    assert!(custom.source_file.is_empty());
    assert!(
        edge(edges, |e| e.source == file_node.id
            && e.target == "networkfactory"
            && e.relation == "references")
        .is_some()
    );

    // H. Imports + exports (global ids, empty source_file)
    let import_node =
        node(nodes, |n| n.id == "package_flutter_material_dart").expect("import node");
    assert!(import_node.source_file.is_empty());
    assert_eq!(import_node.label, "package:flutter/material.dart");

    let export_node =
        node(nodes, |n| n.id == "package_flutter_bloc_flutter_bloc_dart").expect("export node");
    assert!(export_node.source_file.is_empty());
    assert_eq!(export_node.label, "package:flutter_bloc/flutter_bloc.dart");
    assert!(
        edge(edges, |e| e.source == file_node.id
            && e.target == export_node.id
            && e.relation == "exports")
        .is_some()
    );
}

#[test]
fn advanced_dart_features() {
    // The `# ...` section markers are INTENTIONAL and copied byte-for-byte from
    // graphify-py's test_dart.py fixture: `#` is not a Dart comment, so these
    // lines survive comment-stripping and exercise that stray non-`//` lines do
    // not break extraction. Do not "fix" them to `//` (CodeRabbit suggested it;
    // declined — it would change what this fixture tests).
    let code = "
import 'package:riverpod/riverpod.dart';

# 1. Combined Modifiers & Mixin Class
abstract base class MyBaseClass {}
abstract interface class MyInterface {}
mixin class MyMixinClass {}

# 2. Riverpod Functional & Class Providers with Codegen
@riverpod
class MyNotifier extends _$MyNotifier {
  @override
  String build() {
    ref.watch(anotherProvider);
    return \"hello\";
  }
}

@riverpod
String myValue(MyValueRef ref) {
  return \"world\";
}

# 3. Late & Non-Initialized Final Fields
class MyModel {
  late final String lateField;
  final int noInitField;
  final String initField = \"init\";
}

# 4. Records & Pattern Matching in variables
final (int, String) typedRecord = (1, \"one\");
var (recA, recB) = (10, 20);

# 5. Records in method returns & switch expressions
(double, double) getCoordinates() {
    var localVal = switch (typedRecord) {
      (int a, String b) => (1.0, 2.0),
      _ => (0.0, 0.0),
    };
    return localVal;
}

# 6. Bloc constructor event registration & emission
class AuthBloc extends Bloc<AuthEvent, AuthState> {
  AuthBloc() : super(AuthInitial()) {
    on<AuthLogin>((event, emit) {
      emit(AuthLoading());
    });
    on<AuthLogout>((event, emit) {
      yield AuthSuccess();
    });
  }
}

# 7. Widget Bloc trigger & bindings
class HomeWidget {
  void triggerLogin(BuildContext context) {
    context.read<AuthBloc>().add(AuthLogin());
  }
}
";
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = write_dart(tmp.path(), "test_advanced.dart", code);
    let result = extract_dart(&path);
    let nodes = &result.nodes;
    let edges = &result.edges;

    // Classes
    assert!(node(nodes, |n| n.label == "MyBaseClass").is_some());
    assert!(node(nodes, |n| n.label == "MyInterface").is_some());
    assert!(node(nodes, |n| n.label == "MyMixinClass").is_some());
    // No false-positive node literally named "class"
    assert!(node(nodes, |n| n.label == "class").is_none());

    // Late & final fields
    assert!(node(nodes, |n| n.label == "lateField").is_some());
    assert!(node(nodes, |n| n.label == "noInitField").is_some());
    assert!(node(nodes, |n| n.label == "initField").is_some());

    // Records & destructuring
    assert!(node(nodes, |n| n.label == "typedRecord").is_some());
    assert!(node(nodes, |n| n.label == "recA").is_some());
    assert!(node(nodes, |n| n.label == "recB").is_some());
    // Nested switch-expression local is not a top-level define
    assert!(node(nodes, |n| n.label == "localVal").is_none());

    // Record-returning method
    assert!(node(nodes, |n| n.label == "getCoordinates").is_some());

    // Riverpod codegen defines
    assert!(node(nodes, |n| n.label == "myNotifierProvider").is_some());
    assert!(node(nodes, |n| n.label == "myValueProvider").is_some());

    // Riverpod watcher reference
    assert!(
        edge(edges, |e| e.target == "anotherprovider"
            && e.relation == "references")
        .is_some()
    );

    // Bloc constructor events & emissions
    assert!(
        edge(edges, |e| e.target == "authlogin"
            && e.context.as_deref() == Some("bloc_event"))
        .is_some()
    );
    assert!(
        edge(edges, |e| e.target == "authloading"
            && e.context.as_deref() == Some("emit_state"))
        .is_some()
    );

    // Widget Bloc trigger
    assert!(
        edge(edges, |e| e.target == "authlogin"
            && e.context.as_deref() == Some("bloc_add_event"))
        .is_some()
    );
    assert!(
        edge(edges, |e| e.target == "authbloc"
            && e.context.as_deref() == Some("bloc_lookup"))
        .is_some()
    );
}

#[test]
fn namespace_and_spaced_generics() {
    let code = "
class MyWidget extends foo.Bar<Map<String, int>> implements ui.Widget, db.Model {}

final Map<String, int> myVar = 10;
const List<Map<String, int>> myList = [];
late final auth.AuthService authService;

Map<String, Map<String, int>> myMethod(String a) {}
auth.AuthService init() {}
";
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = write_dart(tmp.path(), "test_namespaces.dart", code);
    let result = extract_dart(&path);
    let nodes = &result.nodes;
    let edges = &result.edges;

    let widget = node(nodes, |n| n.label == "MyWidget").expect("MyWidget node");
    let extends =
        edge(edges, |e| e.source == widget.id && e.relation == "inherits").expect("inherits edge");
    assert_ne!(extends.target, "foo", "namespaced base must not be clipped");

    assert!(node(nodes, |n| n.label == "myVar").is_some());
    assert!(node(nodes, |n| n.label == "myList").is_some());
    assert!(node(nodes, |n| n.label == "authService").is_some());
    assert!(node(nodes, |n| n.label == "myMethod").is_some());
    assert!(node(nodes, |n| n.label == "init").is_some());
}

#[test]
fn dart_and_flutter_specifics() {
    let code = "
mixin AuthMixin on BaseWidget {}
typedef JsonMap = Map<String, dynamic>;
extension type UserId(int value) implements Object {}

class MyService {
  final AuthService api;
  MyService(this.api);

  factory MyService.fromJson() {}

  void navigate(BuildContext context) {
    context.go('/home');
    Navigator.pushNamed(context, Routes.login);
    context.router.push(ProfileRoute());
  }
}
";
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = write_dart(tmp.path(), "test_specifics.dart", code);
    let result = extract_dart(&path);
    let nodes = &result.nodes;
    let edges = &result.edges;

    // 1. Mixin `on` relation
    let auth_mixin = node(nodes, |n| n.label == "AuthMixin").expect("AuthMixin node");
    assert!(
        edge(edges, |e| e.source == auth_mixin.id
            && e.relation == "inherits"
            && e.target == "basewidget")
        .is_some()
    );

    // 2. Typedef
    assert!(node(nodes, |n| n.label == "JsonMap").is_some());

    // 3. Variable DI type
    assert!(node(nodes, |n| n.label == "api").is_some());
    assert!(
        edge(edges, |e| e.target == "authservice"
            && e.relation == "references"
            && e.context.as_deref() == Some("variable_type"))
        .is_some()
    );

    // 4. Factory
    assert!(node(nodes, |n| n.label == "fromJson").is_some());

    // 5. Universal navigation
    assert!(
        edge(edges, |e| e.relation == "navigates"
            && e.context.as_deref() == Some("route_path"))
        .is_some()
    );
    assert!(
        edge(edges, |e| e.relation == "navigates"
            && e.context.as_deref() == Some("route_const"))
        .is_some()
    );
    assert!(
        edge(edges, |e| e.relation == "navigates"
            && e.context.as_deref() == Some("route_object"))
        .is_some()
    );

    // 6. Extension type
    let user_id = node(nodes, |n| n.label == "UserId").expect("UserId node");
    assert!(
        edge(edges, |e| e.source == user_id.id
            && e.relation == "implements"
            && e.target == "object")
        .is_some()
    );
}

#[test]
fn roadmap_bug_fixes() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let parent = write_dart(
        tmp.path(),
        "parent_lib.dart",
        "library parent_lib;\npart 'child_part.dart';",
    );
    let child_code = "
part of 'parent_lib.dart';

class ChildClass extends Bloc<Pair<UserEvent, MyState>, State> {}

var User(name: myVar, age: myAge) = user;

void runDI(BuildContext context) {
    final repo = locator<Repository<User>>();
    context.go('/home?id=123&type=auth');
}
";
    let child = write_dart(tmp.path(), "child_part.dart", child_code);
    let result = extract_dart(&child);
    let nodes = &result.nodes;
    let edges = &result.edges;

    // A. Bug D: no child file node
    assert!(node(nodes, |n| n.label == "child_part.dart").is_none());

    // B. defines edge source is the parent file id
    let parent_fid = make_id1(&parent.canonicalize().expect("canon").to_string_lossy());
    let child_class = node(nodes, |n| n.label == "ChildClass").expect("ChildClass node");
    let def_edge = edge(edges, |e| {
        e.target == child_class.id && e.relation == "defines"
    })
    .expect("defines edge");
    assert_eq!(def_edge.source, parent_fid);

    // C. Bug A: safe nested-generic comma split
    assert!(node(nodes, |n| n.id == "pair").is_some());
    assert!(node(nodes, |n| n.id == "state").is_some());
    // No broken comma-split artifacts like "mystate"
    assert!(node(nodes, |n| n.id.contains("mystate")).is_none());

    // D. Bug B: double-generic DI lookup locator<Repository<User>>()
    assert!(node(nodes, |n| n.id == "repository").is_some());

    // E. Bug E: object-destructuring variables
    assert!(node(nodes, |n| n.label == "myVar").is_some());
    assert!(node(nodes, |n| n.label == "myAge").is_some());
    // The destructure keys (name/age) must NOT become variables
    assert!(
        node(nodes, |n| n.label.contains("name")
            || n.label.contains("age"))
        .is_none()
    );

    // F. Bug C: GoRouter query-parameter route mapping
    let nav = edge(edges, |e| {
        e.relation == "navigates" && e.context.as_deref() == Some("route_path")
    })
    .expect("route_path edge");
    assert_eq!(nav.target, "route_home_id_123_type_auth");
}
