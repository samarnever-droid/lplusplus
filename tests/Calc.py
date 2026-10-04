print("operators 1.Add 2. Sub 3. Mutiply 4.divide")
Opreator = int(input("Type a Operator: "))
FN = int(input("Write first number: "))
SN = int(input("Write Second number: "))
if Opreator == 1:
    print(FN + SN)
elif Opreator == 2:
    print(FN - SN)
elif Opreator == 3:
    print(FN * SN)
elif Opreator == 4:
    print(FN / SN)
else:
    print("Invalid operator")