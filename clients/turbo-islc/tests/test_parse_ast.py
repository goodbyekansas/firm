"""Tests for parsing ISL with Turbo ISLC"""
from turbo_islc import ast


def test_module() -> None:
    """Tests for the main function"""
    ast_root = ast.parse(ast.lex("(mod test-moduel)")[0])
    assert isinstance(ast_root, ast.Module)
    assert ast_root.name == "test-moduel"
    assert ast_root.members == []

    ast_root = ast.parse(ast.lex("(mod test-moduel (mod moar-mudel))")[0])
    assert isinstance(ast_root, ast.Module)
    assert ast_root.name == "test-moduel"
    assert len(ast_root.members) == 1

    inner_module = ast_root.members[0]
    assert isinstance(inner_module, ast.Module)
    assert inner_module.name == "moar-mudel"
    assert inner_module.members == []


def test_nested_module() -> None:
    """Test module in module"""
    data = """(mod mudul (mod modul-ception ))"""
    mod = ast.parse(ast.lex(data)[0])
    assert isinstance(mod, ast.Module)
    assert mod.is_root_module()
    sub_module = mod.members[0]
    assert isinstance(sub_module, ast.Module)
    assert sub_module.name == "modul-ception"
    assert not sub_module.is_root_module()


def test_double_module() -> None:
    """Test multiple root modules"""
    data = """
    (mod mod1)
    (mod mod2)"""
    mod1, mod2 = list(map(ast.parse, ast.lex(data)))
    assert isinstance(mod1, ast.Module)
    assert isinstance(mod2, ast.Module)
    assert mod1.name == "mod1"
    assert mod2.name == "mod2"


def test_function() -> None:
    """Test the function type in Turbo ISLC"""
    # Minimalistic example
    data = """
    (mod blobb (fun best-function () ()))
    """
    function = ast.parse(ast.lex(data)[0]).members[0]  # type: ignore
    assert isinstance(function, ast.Function)
    assert function.name == "best-function"
    assert len(function.arguments) == 0
    assert len(function.return_values) == 0

    # Function with arguments
    data = """
    (
        mod guleboj
            (fun arg-function (
                :apa-count int
                :apa-bounces-count int
                :apa-name string) ()))
    """
    function = ast.parse(ast.lex(data)[0]).members[0]  # type: ignore
    assert isinstance(function, ast.Function)
    assert len(function.arguments) == 3
    arg0, arg1, arg2 = list(function.arguments.values())
    assert isinstance(arg0, ast.FunctionArgument)
    assert arg0.name == "apa-count"
    assert arg0.data_type == ast.DataType.INT

    assert isinstance(arg1, ast.FunctionArgument)
    assert arg1.name == "apa-bounces-count"
    assert arg1.data_type == ast.DataType.INT

    assert isinstance(arg2, ast.FunctionArgument)
    assert arg2.name == "apa-name"
    assert arg2.data_type == ast.DataType.STRING

    # Function with return type
    data = """
    (mod trampetapet (fun arg-function () (:sune int :angry bool)))
    """
    function = ast.parse(ast.lex(data)[0]).members[0]  # type: ignore
    assert isinstance(function, ast.Function)
    assert len(function.return_values) == 2

    return_value0, return_value1 = list(function.return_values.values())
    assert isinstance(return_value0, ast.FunctionReturnValue)
    assert return_value0.name == "sune"
    assert return_value0.data_type == ast.DataType.INT

    assert isinstance(return_value1, ast.FunctionReturnValue)
    assert return_value1.name == "angry"
    assert return_value1.data_type == ast.DataType.BOOL


def test_record() -> None:
    """Test the record type in Turbo ISLC"""
    # Record with simple data types
    data = """
    (mod tallefjant (rec spiselapp (:ekollon int :vatten float)))
    """
    record = ast.parse(ast.lex(data)[0]).members[0]  # type: ignore
    assert isinstance(record, ast.Record)
    assert record.name == "spiselapp"
    assert record["ekollon"].data_type == ast.DataType.INT
    assert record["vatten"].data_type == ast.DataType.FLOAT

    # Record with a record
    data = """
    (mod eat
        (rec food (:pasta float :meatballs int))
        (rec dinner (:dish food :wine float)))
    """
    module = ast.parse(ast.lex(data)[0])
    assert isinstance(module, ast.Module)
    dinner = module.record("dinner")
    assert dinner is not None
    dish = dinner["dish"]
    assert dish.is_record()
    assert dish.name == "dish"
    dish2 = dish.as_record()
    assert dish2 is not None
    assert dish2["pasta"].as_datatype() == ast.DataType.FLOAT

    # Record with lists
    data = """
    (mod eat
        (rec food (:leverpanna int :salladsdolmar int :makrillaladab bool))
        (rec order (:dishes (list food) :wine (list float)))
    )
    """
    module = ast.parse(ast.lex(data)[0])
    assert isinstance(module, ast.Module)
    record = module.record("order")
    dishes = record["dishes"]
    assert dishes.is_list()
    assert dishes.is_record()
    assert dishes.as_record()["salladsdolmar"].as_datatype() == ast.DataType.INT


def test_doc_strings() -> None:
    """Test that doc strings are captured."""
    data = """
    (mod lada "i ladan bor kossan"
        (rec ko "🐄 mamma mu" (:liter-mjolk float))
        (fun mjolka "Mjölk ko-olt!" () ()))
    """
    module = ast.parse(ast.lex(data)[0])
    assert isinstance(module, ast.Module)
    assert module.doc_string == "i ladan bor kossan"
    record = module.record("ko")
    assert record is not None
    assert record.doc_string == "🐄 mamma mu"
    fun = module.function("mjolka")
    assert fun is not None
    assert fun.doc_string == "Mjölk ko-olt!"


def test_comments() -> None:
    """Test that comments are ok."""
    data = """
    ;; Crap here
    (mod apa ; more crap here
        ;; :D::D:D:D
        (fun banan (:ja bool) () ); side effect function
        ;; yes
    )"""
    module = ast.parse(ast.lex(data)[0])
    assert isinstance(module, ast.Module)
    assert isinstance(module.function("banan"), ast.Function)


def test_datatypes() -> None:
    """Test different supported data types"""
    # pylint: disable=too-many-statements
    data = """(mod datafiler
        (rec datta (:heltal int :flyttal float :boll bool :text string))
        (rec datta-lista (
            :heltals (list int)
            :flyttals (list float)
            :bolls (list bool)
            :texts (list string)))
        (rec mer-datta (:datta datta :dattas (list datta)))
    )"""
    module = ast.parse(ast.lex(data)[0])
    assert isinstance(module, ast.Module)
    datta = module.record("datta")
    assert isinstance(datta, ast.Record)
    integer = datta["heltal"]
    assert not integer.is_list()
    assert not integer.is_record()
    assert integer.as_datatype() == ast.DataType.INT
    float_num = datta["flyttal"]
    assert not float_num.is_list()
    assert not float_num.is_record()
    assert float_num.as_datatype() == ast.DataType.FLOAT
    boll = datta["boll"]
    assert not boll.is_list()
    assert not boll.is_record()
    assert boll.as_datatype() == ast.DataType.BOOL
    text = datta["text"]
    assert not text.is_list()
    assert not text.is_record()
    assert text.as_datatype() == ast.DataType.STRING

    mer_datta = module.record("datta-lista")
    assert isinstance(mer_datta, ast.Record)
    integer = mer_datta["heltals"]
    assert integer.is_list()
    assert not integer.is_record()
    assert integer.as_datatype() == ast.DataType.INT
    float_num = mer_datta["flyttals"]
    assert float_num.is_list()
    assert not float_num.is_record()
    assert float_num.as_datatype() == ast.DataType.FLOAT
    boll = mer_datta["bolls"]
    assert boll.is_list()
    assert not boll.is_record()
    assert boll.as_datatype() == ast.DataType.BOOL
    text = mer_datta["texts"]
    assert text.is_list()
    assert not text.is_record()
    assert text.as_datatype() == ast.DataType.STRING

    mer_mer_datta = module.record("mer-datta")
    assert isinstance(mer_mer_datta, ast.Record)
    datta2 = mer_mer_datta["datta"]
    assert not datta2.is_list()
    assert datta2.is_record()
    datta2r = datta2.as_record()
    assert datta2r is not None
    assert datta2.name == "datta"

    mer_mer_dattas = module.record("mer-datta")
    assert isinstance(mer_mer_dattas, ast.Record)
    dattas = mer_mer_dattas["dattas"]
    assert dattas.is_list()
    yes = dattas.as_record()
    assert yes is not None
    assert yes.name == "datta"


def test_modifiers() -> None:
    """Test modifiers."""
    data = """(mod references
        (rec record (
                     :name (ref string)
                     :song (list string)
                     :authors (ref list string)))

        (rec paper (
                    :text string
                    :number int))
    )"""

    module = ast.parse(ast.lex(data)[0])
    assert isinstance(module, ast.Module)

    record = module.record("record")
    assert record is not None
    assert record["name"].is_reference()
    assert not record["name"].is_list()

    assert record["song"].is_list()
    assert not record["song"].is_reference()

    assert record["authors"].is_reference()
    assert record["authors"].is_list()

    paper = module.record("paper")
    assert paper is not None
    assert not paper["text"].is_reference()
    assert not paper["text"].is_list()

    assert not paper["number"].is_reference()
    assert not paper["number"].is_list()


def test_bytes() -> None:
    """Test the cool bytes type."""
    data = """(mod references
        (rec record (
            :normal-stuff int
            :stuff bytes
            :more-stuff (ref bytes)
            :even-more-stuff (ref list bytes)
            :text string)))
    """

    module = ast.parse(ast.lex(data)[0])
    assert isinstance(module, ast.Module)

    record = module.record("record")
    assert record is not None
    assert not record["normal-stuff"].is_list()
    # Bytes secretly forces list
    assert record["stuff"].is_list()
    assert record["more-stuff"].is_list()
    assert record["more-stuff"].is_reference()
    assert record["even-more-stuff"].is_list()
    assert record["even-more-stuff"].is_reference()
    assert not record["text"].is_reference()
    assert not record["text"].is_list()


def test_enums() -> None:
    """Test the enum thing."""

    data = """(mod birbs
        (enu bird-type "Different kinds of birds" (
            :vulture
            :albatross
            :eagle
            :seagull))
        (rec bird (:type bird-type :beak-size int :steals-french-fries bool))
        (fun which-bird (:description string) (:bird bird-type))
        (fun wingspan (:bird (ref bird-type)) (:meters float)))"""

    module = ast.parse(ast.lex(data)[0])
    assert isinstance(module, ast.Module)

    enum = module.enum("bird-type")
    assert enum is not None
    assert "albatross" in enum

    fun1 = module.function("which-bird")
    assert fun1 is not None
    ret_val = fun1.get_first_return_value()
    assert ret_val is not None
    assert ret_val.is_enum()
    assert ret_val.as_enum() is not None

    fun2 = module.function("wingspan")
    assert fun2 is not None
    arg = fun2.arguments.get("bird")
    assert arg is not None
    assert arg.is_enum()
    assert arg.as_enum() is not None
    assert arg.is_reference()

    record = module.record("bird")
    assert record is not None
    assert record["type"].is_enum()
