declare i8 @llvm.pext.i8(i8, i8)
declare i8 @llvm.pdep.i8(i8, i8)
define i8 @aes_pext8(i8 %v, i8 %m) { %r = call i8 @llvm.pext.i8(i8 %v, i8 %m) ret i8 %r }
define i8 @aes_pdep8(i8 %v, i8 %m) { %r = call i8 @llvm.pdep.i8(i8 %v, i8 %m) ret i8 %r }
declare i16 @llvm.pext.i16(i16, i16)
declare i16 @llvm.pdep.i16(i16, i16)
define i16 @aes_pext16(i16 %v, i16 %m) { %r = call i16 @llvm.pext.i16(i16 %v, i16 %m) ret i16 %r }
define i16 @aes_pdep16(i16 %v, i16 %m) { %r = call i16 @llvm.pdep.i16(i16 %v, i16 %m) ret i16 %r }
declare i32 @llvm.pext.i32(i32, i32)
declare i32 @llvm.pdep.i32(i32, i32)
define i32 @aes_pext32(i32 %v, i32 %m) { %r = call i32 @llvm.pext.i32(i32 %v, i32 %m) ret i32 %r }
define i32 @aes_pdep32(i32 %v, i32 %m) { %r = call i32 @llvm.pdep.i32(i32 %v, i32 %m) ret i32 %r }
declare i64 @llvm.pext.i64(i64, i64)
declare i64 @llvm.pdep.i64(i64, i64)
define i64 @aes_pext64(i64 %v, i64 %m) { %r = call i64 @llvm.pext.i64(i64 %v, i64 %m) ret i64 %r }
define i64 @aes_pdep64(i64 %v, i64 %m) { %r = call i64 @llvm.pdep.i64(i64 %v, i64 %m) ret i64 %r }
