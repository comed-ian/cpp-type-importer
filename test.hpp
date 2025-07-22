#include <stdint.h>

struct aaa {
  int32_t a;
  int64_t b;
  bool* c;
};

struct bbb {
  aaa innards;
};

struct ccc {
  aaa* a;
};

struct ddd {
  void* (*f)(void*, int32_t);
};

template <typename T, typename UV> struct Abc {
  T* a;
  UV* *b;
};

Abc<int32_t, bool>;

Abc<Abc<int32_t, bool>, int32_t>;

Abc<uint32_t, uint32_t*>;

template <typename A> struct Def {
  Abc<A, A*> a;
};

Def<uint32_t>;

template <typename T> struct eee {
  void* (*fn)(T* clazz);
};

eee<aaa>;

enum fff : uint8_t {
  ZERO,
  ONE,
  TWO
};

enum ggg {
  GGGONE=1,
  GGGTWO,
  GGGTHREE,
};

class HHH {
  HHH();
  HHH(int32_t);
  ~HHH();
  bool myMethod(uint32_t*);
  // ; end vtable
  uint32_t a;
  aaa* b;
  void* (*cb)();
};

class III {
  III();
  ~III();
  bool MyMethod(uint32_t* a); // ; offset=1
  // ; end vtable
  bool arg1;
  char* unk;
};

class JJJ : HHH {
  JJJ(); // ; override void (* HHH_vtable::HHH)(struct HHH* this);
  ~JJJ(); // ; override void (* HHH_vtable::~HHH)(struct HHH* this);
  // ; end vtable
  uint32_t jjj_member;
  uint32_t _pad;
};

class KK : HHH, III {
  // ; end vtable
  uint32_t kk_member;
  uint32_t _pad;
};

class LLL {
  LLL();
  ~LLL();
  bool testNum(ggg num); // ; offset=1
  // ; end vtable
  ggg num;
  uint32_t _pad;
};

class MMM : HHH, III, LLL {
  uint32_t getVal();
  // ; end vtable
  uint32_t MMM_val;
  uint32_t _pad;
  bool arg2; // ; override bool III::arg1;
  void** (*cb2)(); // ; override void* (* HHH::cb)();
};

typedef MMM NNN;

class SSS  {
    SSS();
    void Sfunc(uint32_t);
    int Sfunc2(uint64_t);
    // ; end vtable
    bool s_member;
    char _pad0;
    char _pad1;
    char _pad2;
    uint32_t _pad3;
};

class TTT {
  void* do_it(void* arg);
  // ; end vtable
  void* a;
  void* b;
};

class UUU: SSS, LLL, TTT {
    UUU();
    // ; end vtable
    int32_t u_member;
    uint32_t _pad;
};

class VVV: KK, UUU {
   // ; end vtable
   int64_t v_member;
};

typedef Abc<int32_t, bool>* OOO;

struct __attribute__((packed)) PPP {
    bool a;
    uint32_t b;
    char* c;
    aaa d;
};

class Level1 {
    Level1();
    ~Level1();
    void virtualMethod();
    // ; end vtable
    int8_t level1_data;
};

class Level2 : Level1 {
    Level2(); // ; override void (* Level1_vtable::Level1)(struct Level1* this);
    ~Level2(); // ; override void (* Level1_vtable::~Level1)(struct Level1* this);
    void virtualMethod(); // ; override void (* Level1_vtable::virtualMethod)(struct Level1* this);
    void level2Method();
    // ; end vtable
    int16_t level2_data;
};

class Level3 : Level2 {
    Level3(); // ; override void (* Level2_vtable_Level1::Level2)(struct Level2* this);
    ~Level3(); // ; override void (* Level2_vtable_Level1::~Level2)(struct Level2* this);
    void level2Method(); // ; override void (* Level2_vtable_Level1::level2Method)(struct Level2* this);
    void level3Method();
    // ; end vtable
    int32_t level3_data;
};

class Level4 : Level3 {
    Level4(); // ; override void (* Level3_vtable_Level2::Level3)(struct Level3* this);
    ~Level4(); // ; override void (* Level3_vtable_Level2::~Level3)(struct Level3* this);
    void level3Method(); // ; override void (* Level3_vtable_Level2::level3Method)(struct Level3* this);
    void level4Method();
    // ; end vtable
    int64_t level4_data;
};

class Base1 {
    Base1();
    ~Base1();
    void method1();
    // ; end vtable
    int32_t base1_member;
    int32_t base1_member2;
};

class Base2 {
    Base2();
    ~Base2();
    void method2();
    // ; end vtable
    int64_t base2_member;
};

class Base3 {
    Base3();
    ~Base3();
    void method3();
    // ; end vtable
    uint32_t base3_member;
    uint32_t base3_member2;
};

class Base4 {
    Base4();
    ~Base4();
    void method4();
    // ; end vtable
    uint64_t base4_member;
};

class MultiMiddle1 : Base1, Base2 {
    MultiMiddle1(); // ; override void (* Base1_vtable::Base1)(struct Base1* this);
    ~MultiMiddle1(); // ; override void (* Base1_vtable::~Base1)(struct Base1* this);
    void MultiMiddle1_constructor(); // ; override void (* Base2_vtable::Base2)(struct Base2* this);
    void MultiMiddle1_destructor(); // ; override void (* Base2_vtable::~Base2)(struct Base2* this);
    void method1(); // ; override void (* Base1_vtable::method1)(struct Base1* this);
    void methodMiddle1();
    // ; end vtable
    uint64_t middle1_member;
    uint64_t base2_member; // ; override int64_t Base2::base2_member;
};

class MultiMiddle2 : Base3, Base4 {
    MultiMiddle2(); // ; override void (* Base3_vtable::Base3)(struct Base3* this);
    ~MultiMiddle2(); // ; override void (* Base3_vtable::~Base3)(struct Base3* this);
    void MultiMiddle2_constructor(); // ; override void (* Base4_vtable::Base4)(struct Base4* this);
    void MultiMiddle2_destructor(); // ; override void (* Base4_vtable::~Base4)(struct Base4* this);
    void method4(); // ; override void (* Base4_vtable::method4)(struct Base4* this);
    void methodMiddle2();
    // ; end vtable
    int64_t middle2_member;
};

class MultiDerived : MultiMiddle1, MultiMiddle2 {
    MultiDerived(); // ; override void (* MultiMiddle1_vtable_Base1::MultiMiddle1)(struct MultiMiddle1* this);
    ~MultiDerived(); // ; override void (* MultiMiddle1_vtable_Base1::~MultiMiddle1)(struct MultiMiddle1* this);
    void MultiDerived_constructor1(); // ; override void (* MultiMiddle1_vtable_Base2::MultiMiddle1_constructor)(struct MultiMiddle1* this);
    void MultiDerived_destructor1(); // ; override void (* MultiMiddle1_vtable_Base2::MultiMiddle1_destructor)(struct MultiMiddle1* this);
    void MultiDerived_constructor2(); // ; override void (* MultiMiddle2_vtable_Base3::MultiMiddle2)(struct MultiMiddle2* this);
    void MultiDerived_destructor2(); // ; override void (* MultiMiddle2_vtable_Base3::~MultiMiddle2)(struct MultiMiddle2* this);
    void MultiDerived_constructor3(); // ; override void (* MultiMiddle2_vtable_Base4::MultiMiddle2_constructor)(struct MultiMiddle2* this);
    void MultiDerived_destructor3(); // ; override void (* MultiMiddle2_vtable_Base4::MultiMiddle2_destructor)(struct MultiMiddle2* this);
    void methodMiddle1(); // ; override void (* MultiMiddle1_vtable_Base1::methodMiddle1)(struct MultiMiddle1* this);
    void methodDerived();
    void method4(); // ; override void (* Base4_vtable::method4)(struct Base4* this);
    // ; end vtable
    int64_t base4_member; // ; override uint64_t Base4::base4_member;
    bool derived_member;
};
 
namespace QQQ {
    struct aaa {
        uint32_t a;
        void* b;
    };

    namespace RRR {
        struct aaa {
            void* a;
            uint32_t b;
        };

        struct bbb {
            aaa* a;
        };

        struct ccc {
            QQQ::aaa a;
        };
    } // end RRR
} // end QQQ

struct WWW {
    uint64_t a;
    char b[0x10];
    uint32_t c[0x4];
};

struct XXX {
    WWW a[0x3];
    char* b[0x4];
    int32_t c[0x8];
};
